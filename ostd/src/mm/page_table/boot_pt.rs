use core::{
    alloc::Layout,
    sync::atomic::{AtomicU32, Ordering},
};

use ostd_pod::FromZeros;

use super::{PteTrait, pte_index};
use crate::{
    arch::mm::{PageTableEntry, PagingConsts},
    cpu::num_cpus,
    cpu_local_cell,
    mm::{
        Frame, FrameAllocOptions, PAGE_SIZE, Paddr, PageProperty, PagingConstsTrait, PagingLevel,
        Vaddr,
        frame::{
            self,
            allocator::{self, EarlyAllocatedFrameMeta},
        },
        nr_subpage_per_huge, paddr_to_vaddr,
        page_prop::PageTableFlags,
        page_table::PteScalar,
    },
    sync::SpinLock,
};

pub(crate) fn with_borrow<F, R>(f: F) -> Result<R, ()>
where
    F: FnOnce(&mut BootPageTable) -> R,
{
    let mut boot_pt = BOOT_PAGE_TABLE.lock();

    if IS_DISMISSED.load() {
        return Err(());
    }

    if boot_pt.is_none() {
        *boot_pt = Some(unsafe { BootPageTable::from_current_pt() });
    }

    let r = f(boot_pt.as_mut().unwrap());

    Ok(r)
}

pub(crate) unsafe fn dismiss() {
    IS_DISMISSED.store(true);
    if DISMISS_COUNT.fetch_add(1, Ordering::AcqRel) as usize == num_cpus() - 1 {
        let boot_pt = BOOT_PAGE_TABLE.lock().take().unwrap();

        dfs_walk_on_leave::<PageTableEntry, PagingConsts>(
            boot_pt.root_pt,
            PagingConsts::NR_LEVELS,
            &mut |pte, pa, _, flags| {
                if !flags.contains(PTE_POINTS_TO_FIRMWARE_PT) {
                    drop(unsafe { Frame::<EarlyAllocatedFrameMeta>::from_raw(pa) })
                }
                *pte = PageTableEntry::new_zeroed();
            },
        );
    }
}

static BOOT_PAGE_TABLE: SpinLock<Option<BootPageTable>> = SpinLock::new(None);
static DISMISS_COUNT: AtomicU32 = AtomicU32::new(0);
cpu_local_cell! {
    static IS_DISMISSED: bool = false;
}

pub(crate) struct BootPageTable<E: PteTrait = PageTableEntry, C: PagingConstsTrait = PagingConsts> {
    root_pt: Paddr,
    _phantom: core::marker::PhantomData<(E, C)>,
}

const PTE_POINTS_TO_FIRMWARE_PT: PageTableFlags = PageTableFlags::AVAIL1;

impl<E: PteTrait, C: PagingConstsTrait> BootPageTable<E, C> {
    unsafe fn from_current_pt() -> Self {
        let root_pt = crate::arch::mm::current_page_table_paddr();
        dfs_walk_on_leave::<E, C>(
            root_pt,
            C::NR_LEVELS,
            &mut |pte: &mut E, pa, level, mut flags| {
                flags |= PTE_POINTS_TO_FIRMWARE_PT;
                *pte = E::from_repr(&PteScalar::PageTable(pa, flags), level);
            },
        );
        Self {
            root_pt,
            _phantom: core::marker::PhantomData,
        }
    }

    pub(crate) fn root_address(&self) -> Paddr {
        self.root_pt
    }

    pub(in crate::mm) unsafe fn map_base_page(
        &mut self,
        from: Vaddr,
        to: Paddr,
        prop: PageProperty,
    ) {
        let mut pt = self.root_pt;
        let mut level = C::NR_LEVELS;
        while level > 1 {
            let index = pte_index::<C>(from, level);
            let pte_ptr = unsafe { (paddr_to_vaddr(pt) as *mut E).add(index) };
            let pte = unsafe { pte_ptr.read() };
            match pte.to_repr(level) {
                PteScalar::Absent => {
                    let (pte, child_pt) = self.alloc_child(level);
                    unsafe { pte_ptr.write(pte) };
                    pt = child_pt;
                }
                PteScalar::Mapped(_, _) => {
                    panic!("mapping an already mapped huge page in the boot page table");
                }
                PteScalar::PageTable(child_pt, _) => {
                    pt = child_pt;
                }
            };
            level -= 1;
        }
        let index = pte_index::<C>(from, 1);
        let pte_ptr = unsafe { (paddr_to_vaddr(pt) as *mut E).add(index) };
        let pte = unsafe { pte_ptr.read() };
        if matches!(pte.to_repr(1), PteScalar::Mapped(_, _)) {
            panic!("mapping an already mapped page in the boot page table");
        }
        unsafe { pte_ptr.write(E::from_repr(&PteScalar::Mapped(to, prop), 1)) };
    }

    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(super) unsafe fn protect_base_page(
        &mut self,
        virt_addr: Vaddr,
        mut op: impl FnMut(&mut PageProperty),
    ) {
        let mut pt = self.root_pt;
        let mut level = C::NR_LEVELS;
        while level > 1 {
            let index = pte_index::<C>(virt_addr, level);
            let pte_ptr = unsafe { (paddr_to_vaddr(pt) as *mut E).add(index) };
            let pte = unsafe { pte_ptr.read() };
            match pte.to_repr(level) {
                PteScalar::Absent => {
                    panic!("protecting an unmapped page in the boot page table");
                }
                PteScalar::PageTable(child_pt, _) => {
                    pt = child_pt;
                }
                PteScalar::Mapped(huge_pa, prop) => {
                    let (child_pte, child_frame_pa) = self.alloc_child(level);
                    for i in 0..nr_subpage_per_huge::<C>() {
                        let nxt_ptr = unsafe { (paddr_to_vaddr(child_frame_pa) as *mut E).add(i) };
                        unsafe {
                            nxt_ptr.write(E::from_repr(
                                &PteScalar::Mapped(huge_pa + i * C::BASE_PAGE_SIZE, prop),
                                level - 1,
                            ))
                        };
                    }
                    unsafe { pte_ptr.write(child_pte) };
                    pt = child_frame_pa;
                }
            };
            level -= 1;
        }
        let index = pte_index::<C>(virt_addr, 1);
        let pte_ptr = unsafe { (paddr_to_vaddr(pt) as *mut E).add(index) };
        let pte = unsafe { pte_ptr.read() };
        let PteScalar::Mapped(pa, mut prop) = pte.to_repr(1) else {
            panic!("protecting an unmapped page in the boot page table");
        };
        op(&mut prop);
        unsafe { pte_ptr.write(E::from_repr(&PteScalar::Mapped(pa, prop), 1)) };
    }

    fn alloc_child(&mut self, level: PagingLevel) -> (E, Paddr) {
        let frame_paddr = if frame::meta::is_initialized() {
            let frame = FrameAllocOptions::new()
                .zeroed(false)
                .alloc_frame_with(EarlyAllocatedFrameMeta)
                .unwrap();
            frame.into_raw()
        } else {
            allocator::early_alloc(
                Layout::from_size_align(C::BASE_PAGE_SIZE, C::BASE_PAGE_SIZE).unwrap(),
            )
            .unwrap()
        };

        let vaddr = paddr_to_vaddr(frame_paddr) as *mut u8;
        unsafe { core::ptr::write_bytes(vaddr, 0, PAGE_SIZE) };

        (
            E::from_repr(
                &PteScalar::PageTable(frame_paddr, PageTableFlags::empty()),
                level,
            ),
            frame_paddr,
        )
    }

    #[cfg(ktest)]
    pub(super) fn new(root_pt: Paddr) -> Self {
        Self {
            root_pt,
            _phantom: core::marker::PhantomData,
        }
    }
}

fn dfs_walk_on_leave<E: PteTrait, C: PagingConstsTrait>(
    pt: Paddr,
    level: PagingLevel,
    op: &mut impl FnMut(&mut E, Paddr, PagingLevel, PageTableFlags),
) {
    if level >= 2 {
        let pt_vaddr = paddr_to_vaddr(pt) as *mut E;
        for offset in 0..nr_subpage_per_huge::<C>() {
            let pte_ptr = unsafe { pt_vaddr.add(offset) };
            let mut pte = unsafe { pte_ptr.read() };

            if let PteScalar::PageTable(child_pt, flags) = pte.to_repr(level) {
                dfs_walk_on_leave::<E, C>(child_pt, level - 1, op);
                op(&mut pte, child_pt, level, flags);
                unsafe { pte_ptr.write(pte) };
            }
        }
    }
}
