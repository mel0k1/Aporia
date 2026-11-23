#![cfg_attr(
    any(
        target_arch = "riscv64",
        target_arch = "loongarch64",
        target_arch = "aarch64"
    ),
    expect(dead_code)
)]

use core::{
    fmt::Debug,
    intrinsics::transmute_unchecked,
    ops::{Range, RangeInclusive},
    sync::atomic::{AtomicUsize, Ordering},
};

use ostd_pod::Pod;

use super::{
    HasPaddr, Paddr, PagingConstsTrait, PagingLevel, PodOnce, Vaddr, kspace::KernelPtConfig,
    nr_subpage_per_huge, page_prop::PageProperty, page_size, vm_space::UserPtConfig,
};
use crate::{
    arch::mm::{PageTableEntry, PagingConsts},
    mm::page_prop::PageTableFlags,
    task::{atomic_mode::AsAtomicModeGuard, disable_preempt},
};

mod cursor;
mod node;

pub(crate) use cursor::{Cursor, CursorMut, PageTableFrag};
use node::*;

#[cfg(ktest)]
mod test;

pub(crate) mod boot_pt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PageTableError {
    InvalidVaddrRange(Vaddr, Vaddr),
    InvalidVaddr(Vaddr),
    UnalignedVaddr,
}

pub(crate) unsafe trait PageTableConfig:
    Clone + Debug + Send + Sync + 'static
{
    const TOP_LEVEL_INDEX_RANGE: Range<usize>;

    const TOP_LEVEL_CAN_UNMAP: bool = true;

    type E: PteTrait;

    type C: PagingConstsTrait;

    type Item: Send;
    type ItemRef<'a>;

    fn item_raw_info(item: &Self::Item) -> (Paddr, PagingLevel, PageProperty);

    fn item_into_raw(item: Self::Item) -> (Paddr, PagingLevel, PageProperty) {
        let item = core::mem::ManuallyDrop::new(item);
        Self::item_raw_info(&*item)
    }

    unsafe fn item_from_raw(paddr: Paddr, level: PagingLevel, prop: PageProperty) -> Self::Item;

    unsafe fn item_ref_from_raw<'a>(
        paddr: Paddr,
        level: PagingLevel,
        prop: PageProperty,
    ) -> Self::ItemRef<'a>;
}

impl<C: PageTableConfig> PagingConstsTrait for C {
    const BASE_PAGE_SIZE: usize = C::C::BASE_PAGE_SIZE;
    const NR_LEVELS: PagingLevel = C::C::NR_LEVELS;
    const HIGHEST_TRANSLATION_LEVEL: PagingLevel = C::C::HIGHEST_TRANSLATION_LEVEL;
    const PTE_SIZE: usize = C::C::PTE_SIZE;
    const ADDRESS_WIDTH: usize = C::C::ADDRESS_WIDTH;
    const VA_SIGN_EXT: bool = C::C::VA_SIGN_EXT;
}

pub(crate) fn max_page_level<C: PagingConstsTrait>(len: usize) -> PagingLevel {
    (1..=C::HIGHEST_TRANSLATION_LEVEL)
        .rev()
        .find(|&level| page_size::<C>(level) <= len)
        .unwrap_or(1)
}

pub(crate) fn largest_pages<C: PageTableConfig>(
    mut va: Vaddr,
    mut pa: Paddr,
    mut len: usize,
) -> impl Iterator<Item = (Paddr, PagingLevel)> {
    assert_eq!(va % C::BASE_PAGE_SIZE, 0);
    assert_eq!(pa % C::BASE_PAGE_SIZE, 0);
    assert_eq!(len % C::BASE_PAGE_SIZE, 0);
    assert!(is_valid_range::<C>(&(va..(va + len))));

    core::iter::from_fn(move || {
        if len == 0 {
            return None;
        }

        let mut level = C::HIGHEST_TRANSLATION_LEVEL;
        while page_size::<C>(level) > len
            || !va.is_multiple_of(page_size::<C>(level))
            || !pa.is_multiple_of(page_size::<C>(level))
        {
            level -= 1;
        }

        let item_start = pa;
        va += page_size::<C>(level);
        pa += page_size::<C>(level);
        len -= page_size::<C>(level);

        Some((item_start, level))
    })
}

pub(super) const fn vaddr_range<C: PageTableConfig>() -> RangeInclusive<Vaddr> {
    const fn top_level_index_width<C: PageTableConfig>() -> usize {
        C::ADDRESS_WIDTH - pte_index_bit_offset::<C>(C::NR_LEVELS)
    }

    const {
        assert!(C::TOP_LEVEL_INDEX_RANGE.start < C::TOP_LEVEL_INDEX_RANGE.end);
        assert!(top_level_index_width::<C>() <= nr_pte_index_bits::<C>(),);
        assert!(C::TOP_LEVEL_INDEX_RANGE.start < 1 << top_level_index_width::<C>());
        assert!(C::TOP_LEVEL_INDEX_RANGE.end <= 1 << top_level_index_width::<C>());
    };

    const fn pt_va_range_start<C: PageTableConfig>() -> Vaddr {
        C::TOP_LEVEL_INDEX_RANGE.start << pte_index_bit_offset::<C>(C::NR_LEVELS)
    }

    const fn pt_va_range_end<C: PageTableConfig>() -> Vaddr {
        C::TOP_LEVEL_INDEX_RANGE
            .end
            .unbounded_shl(pte_index_bit_offset::<C>(C::NR_LEVELS) as u32)
            .wrapping_sub(1)
    }

    const fn sign_bit_of_va<C: PageTableConfig>(va: Vaddr) -> bool {
        (va >> (C::ADDRESS_WIDTH - 1)) & 1 != 0
    }

    let mut start = pt_va_range_start::<C>();
    let mut end = pt_va_range_end::<C>();

    const {
        assert!(
            !C::VA_SIGN_EXT
                || sign_bit_of_va::<C>(pt_va_range_start::<C>())
                    == sign_bit_of_va::<C>(pt_va_range_end::<C>()),
            "The sign bit of both range endpoints must be the same if sign extension is enabled"
        )
    }

    if C::VA_SIGN_EXT && sign_bit_of_va::<C>(pt_va_range_start::<C>()) {
        start |= !0 ^ ((1 << C::ADDRESS_WIDTH) - 1);
        end |= !0 ^ ((1 << C::ADDRESS_WIDTH) - 1);
    }

    start..=end
}

const fn is_valid_range<C: PageTableConfig>(r: &Range<Vaddr>) -> bool {
    let va_range = vaddr_range::<C>();
    r.start < r.end && (*va_range.start() <= r.start && r.end - 1 <= *va_range.end())
}

const fn nr_pte_index_bits<C: PagingConstsTrait>() -> usize {
    nr_subpage_per_huge::<C>().ilog2() as usize
}

const fn pte_index<C: PagingConstsTrait>(va: Vaddr, level: PagingLevel) -> usize {
    (va >> pte_index_bit_offset::<C>(level)) & (nr_subpage_per_huge::<C>() - 1)
}

const fn pte_index_bit_offset<C: PagingConstsTrait>(level: PagingLevel) -> usize {
    C::BASE_PAGE_SIZE.ilog2() as usize + nr_pte_index_bits::<C>() * (level as usize - 1)
}

#[derive(Debug)]
pub(crate) struct PageTable<C: PageTableConfig> {
    root: PageTableNode<C>,
}

impl PageTable<UserPtConfig> {
    pub(in crate::mm) fn activate(&self) {
        unsafe {
            self.root.activate();
        }
    }
}

impl PageTable<KernelPtConfig> {
    pub(in crate::mm) fn new_kernel_page_table() -> Self {
        let kpt = Self::empty();

        {
            let preempt_guard = disable_preempt();
            let mut root_node = kpt.root.borrow().lock(&preempt_guard);

            for i in KernelPtConfig::TOP_LEVEL_INDEX_RANGE {
                let mut root_entry = root_node.entry(i);
                let _ = root_entry.alloc_if_none(&preempt_guard).unwrap();
            }
        }

        kpt
    }

    pub(in crate::mm) fn create_user_page_table(&'static self) -> PageTable<UserPtConfig> {
        let new_root = PageTableNode::alloc(PagingConsts::NR_LEVELS);

        let preempt_guard = disable_preempt();
        let mut root_node = self.root.borrow().lock(&preempt_guard);
        let mut new_node = new_root.borrow().lock(&preempt_guard);

        const {
            assert!(!KernelPtConfig::TOP_LEVEL_CAN_UNMAP);
            assert!(
                UserPtConfig::TOP_LEVEL_INDEX_RANGE.end
                    <= KernelPtConfig::TOP_LEVEL_INDEX_RANGE.start
            );
        }

        for i in KernelPtConfig::TOP_LEVEL_INDEX_RANGE {
            let root_entry = root_node.entry(i);
            let child = root_entry.to_ref();
            let PteStateRef::PageTable(pt) = child else {
                panic!("The kernel page table doesn't contain shared nodes");
            };

            let pt_addr = pt.paddr();
            let pte = PageTableEntry::from_repr(
                &PteScalar::PageTable(pt_addr, PageTableFlags::empty()),
                UserPtConfig::NR_LEVELS,
            );
            unsafe { new_node.write_pte(i, pte) };
        }
        drop(new_node);

        PageTable::<UserPtConfig> { root: new_root }
    }
}

impl<C: PageTableConfig> PageTable<C> {
    pub(crate) fn empty() -> Self {
        PageTable {
            root: PageTableNode::<C>::alloc(C::NR_LEVELS),
        }
    }

    pub(in crate::mm) unsafe fn first_activate_unchecked(&self) {
        unsafe { self.root.first_activate() };
    }

    pub(crate) fn root_paddr(&self) -> Paddr {
        self.root.paddr()
    }

    #[cfg(ktest)]
    pub(in crate::mm) fn page_walk(&self, vaddr: Vaddr) -> Option<(Paddr, PageProperty)> {
        unsafe { page_walk::<C>(self.root_paddr(), vaddr) }
    }

    pub(crate) fn cursor_mut<'rcu, G: AsAtomicModeGuard>(
        &'rcu self,
        guard: &'rcu G,
        va: &Range<Vaddr>,
    ) -> Result<CursorMut<'rcu, C>, PageTableError> {
        self.cursor_mut_with_min_level(guard, va, 1)
    }

    pub(crate) fn cursor_mut_with_min_level<'rcu, G: AsAtomicModeGuard>(
        &'rcu self,
        guard: &'rcu G,
        va: &Range<Vaddr>,
        min_level: PagingLevel,
    ) -> Result<CursorMut<'rcu, C>, PageTableError> {
        CursorMut::new(self, guard.as_atomic_mode_guard(), va, min_level)
    }

    pub(crate) fn cursor<'rcu, G: AsAtomicModeGuard>(
        &'rcu self,
        guard: &'rcu G,
        va: &Range<Vaddr>,
    ) -> Result<Cursor<'rcu, C>, PageTableError> {
        Cursor::new(self, guard.as_atomic_mode_guard(), va, 1)
    }

    pub(crate) unsafe fn shallow_copy(&self) -> Self {
        PageTable {
            root: self.root.clone(),
        }
    }
}

#[cfg(ktest)]
pub(super) unsafe fn page_walk<C: PageTableConfig>(
    root_paddr: Paddr,
    vaddr: Vaddr,
) -> Option<(Paddr, PageProperty)> {
    use super::paddr_to_vaddr;

    let _rcu_guard = disable_preempt();

    let mut pt_addr = paddr_to_vaddr(root_paddr);
    for cur_level in (1..=C::NR_LEVELS).rev() {
        let offset = pte_index::<C>(vaddr, cur_level);
        let cur_pte = unsafe { load_pte((pt_addr as *mut C::E).add(offset), Ordering::Acquire) };

        match cur_pte.to_repr(cur_level) {
            PteScalar::Absent => return None,
            PteScalar::PageTable(next_pt_addr, _) => {
                pt_addr = paddr_to_vaddr(next_pt_addr);
                continue;
            }
            PteScalar::Mapped(frame_paddr, prop) => {
                debug_assert!(cur_level <= C::HIGHEST_TRANSLATION_LEVEL);
                return Some((
                    frame_paddr + (vaddr & (page_size::<C>(cur_level) - 1)),
                    prop,
                ));
            }
        }
    }

    unreachable!("All present PTEs at the level 1 must be last-level PTEs");
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PteScalar {
    Absent,
    PageTable(Paddr, PageTableFlags),
    Mapped(Paddr, PageProperty),
}

pub(crate) unsafe trait PteTrait:
    Clone + Copy + Debug + Pod + PodOnce + Sized + Send + Sync + 'static
{
    fn from_repr(repr: &PteScalar, level: PagingLevel) -> Self;

    fn to_repr(&self, level: PagingLevel) -> PteScalar;

    fn as_usize(self) -> usize {
        const { assert!(size_of::<Self>() == size_of::<usize>()) };

        unsafe { transmute_unchecked(self) }
    }

    fn from_usize(pte_raw: usize) -> Self {
        const { assert!(size_of::<Self>() == size_of::<usize>()) };

        unsafe { transmute_unchecked(pte_raw) }
    }
}

unsafe fn load_pte<E: PteTrait>(ptr: *mut E, ordering: Ordering) -> E {
    let atomic = unsafe { AtomicUsize::from_ptr(ptr.cast()) };
    let pte_raw = atomic.load(ordering);
    E::from_usize(pte_raw)
}

unsafe fn store_pte<E: PteTrait>(ptr: *mut E, new_val: E, ordering: Ordering) {
    let new_raw = new_val.as_usize();
    let atomic = unsafe { AtomicUsize::from_ptr(ptr.cast()) };
    atomic.store(new_raw, ordering)
}
