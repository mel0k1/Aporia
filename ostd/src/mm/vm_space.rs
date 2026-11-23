use core::{ops::Range, sync::atomic::Ordering};

use super::{AnyUFrameMeta, PagingLevel, page_table::PageTableConfig};
use crate::{
    Error,
    arch::mm::{PageTableEntry, PagingConsts, current_page_table_paddr},
    cpu::{AtomicCpuSet, CpuSet, PinCurrentCpu, local::CpuLocalPtr},
    cpu_local_cell,
    io::IoMem,
    mm::{
        Frame, HasPaddrRange, PAGE_SIZE, PageProperty, PrivilegedPageFlags, UFrame, VmReader,
        VmWriter,
        frame::FrameRef,
        io::Fallible,
        kspace::KERNEL_PAGE_TABLE,
        page_prop::{CachePolicy, PageFlags},
        page_table::{self, PageTable, PageTableFrag},
        tlb::{TlbFlushOp, TlbFlusher},
    },
    prelude::*,
    sync::{RcuDrop, SpinLock},
    task::{DisabledPreemptGuard, atomic_mode::AsAtomicModeGuard, disable_preempt},
};

#[derive(Debug)]
pub struct VmSpace {
    pt: PageTable<UserPtConfig>,
    cpus: AtomicCpuSet,
    iomems: SpinLock<Vec<IoMem>>,
}

impl VmSpace {
    pub fn new() -> Self {
        Self {
            pt: KERNEL_PAGE_TABLE.get().unwrap().create_user_page_table(),
            cpus: AtomicCpuSet::new(CpuSet::new_empty()),
            iomems: SpinLock::new(Vec::new()),
        }
    }

    pub fn cursor<'a, G: AsAtomicModeGuard>(
        &'a self,
        guard: &'a G,
        va: &Range<Vaddr>,
    ) -> Result<Cursor<'a>> {
        Ok(Cursor(self.pt.cursor(guard, va)?))
    }

    pub fn cursor_mut<'a, G: AsAtomicModeGuard>(
        &'a self,
        guard: &'a G,
        va: &Range<Vaddr>,
    ) -> Result<CursorMut<'a>> {
        Ok(CursorMut {
            pt_cursor: self.pt.cursor_mut(guard, va)?,
            flusher: TlbFlusher::new(&self.cpus, disable_preempt()),
            vmspace: self,
        })
    }

    pub fn activate(self: &Arc<Self>) {
        let preempt_guard = disable_preempt();
        let cpu = preempt_guard.current_cpu();

        let last_ptr = ACTIVATED_VM_SPACE.load().as_ptr();

        if last_ptr == Arc::as_ptr(self) {
            return;
        }

        self.cpus.add(cpu, Ordering::Acquire);

        let self_ptr = Arc::into_raw(Arc::clone(self)) as *mut VmSpace;
        ACTIVATED_VM_SPACE.store(CpuLocalPtr::new(self_ptr));

        if !last_ptr.is_null() {
            let last = unsafe { Arc::from_raw(last_ptr) };
            last.cpus.remove(cpu, Ordering::Relaxed);
        }

        self.pt.activate();
    }

    pub fn reader(&self, vaddr: Vaddr, len: usize) -> Result<VmReader<'_, Fallible>> {
        if current_page_table_paddr() != self.pt.root_paddr()
            || !super::is_in_user_space(vaddr, len)
        {
            return Err(Error::AccessDenied);
        }

        Ok(unsafe { VmReader::<Fallible>::from_user_space(vaddr as *const u8, len) })
    }

    pub fn writer(&self, vaddr: Vaddr, len: usize) -> Result<VmWriter<'_, Fallible>> {
        if current_page_table_paddr() != self.pt.root_paddr()
            || !super::is_in_user_space(vaddr, len)
        {
            return Err(Error::AccessDenied);
        }

        Ok(unsafe { VmWriter::<Fallible>::from_user_space(vaddr as *mut u8, len) })
    }

    pub fn reader_writer(
        &self,
        vaddr: Vaddr,
        len: usize,
    ) -> Result<(VmReader<'_, Fallible>, VmWriter<'_, Fallible>)> {
        if current_page_table_paddr() != self.pt.root_paddr()
            || !super::is_in_user_space(vaddr, len)
        {
            return Err(Error::AccessDenied);
        }

        let reader = unsafe { VmReader::<Fallible>::from_user_space(vaddr as *const u8, len) };

        let writer = unsafe { VmWriter::<Fallible>::from_user_space(vaddr as *mut u8, len) };

        Ok((reader, writer))
    }
}

impl Default for VmSpace {
    fn default() -> Self {
        Self::new()
    }
}

impl VmSpace {
    fn find_iomem_by_paddr(&self, paddr: Paddr) -> Option<(IoMem, usize)> {
        let iomems = self.iomems.lock();
        for iomem in iomems.iter() {
            let start = iomem.paddr();
            let end = start + iomem.size();
            if paddr >= start && paddr < end {
                let offset = paddr - start;
                return Some((iomem.clone(), offset));
            }
        }
        None
    }
}

pub struct Cursor<'a>(page_table::Cursor<'a, UserPtConfig>);

impl Cursor<'_> {
    pub fn query(&mut self) -> Result<(Range<Vaddr>, Option<VmQueriedItem<'_>>)> {
        let (range, item) = self.0.query()?;
        Ok((range, item.map(VmQueriedItem::from)))
    }

    pub fn find_next(&mut self, len: usize) -> Option<Vaddr> {
        self.0.find_next(len)
    }

    pub fn jump(&mut self, va: Vaddr) -> Result<()> {
        self.0.jump(va)?;
        Ok(())
    }

    pub fn virt_addr(&self) -> Vaddr {
        self.0.virt_addr()
    }
}

pub struct CursorMut<'a> {
    pt_cursor: page_table::CursorMut<'a, UserPtConfig>,
    flusher: TlbFlusher<'a, DisabledPreemptGuard>,
    vmspace: &'a VmSpace,
}

impl<'a> CursorMut<'a> {
    pub fn query(&mut self) -> Result<(Range<Vaddr>, Option<VmQueriedItem<'_>>)> {
        let (range, item) = self.pt_cursor.query()?;
        Ok((range, item.map(VmQueriedItem::from)))
    }

    pub fn find_next(&mut self, len: usize) -> Option<Vaddr> {
        self.pt_cursor.find_next(len)
    }

    pub fn jump(&mut self, va: Vaddr) -> Result<()> {
        self.pt_cursor.jump(va)?;
        Ok(())
    }

    pub fn virt_addr(&self) -> Vaddr {
        self.pt_cursor.virt_addr()
    }

    pub fn flusher(&mut self) -> &mut TlbFlusher<'a, DisabledPreemptGuard> {
        &mut self.flusher
    }

    pub fn map(&mut self, frame: UFrame, prop: PageProperty) {
        let item = VmItem::new_tracked(frame, prop);

        unsafe { self.pt_cursor.map(item) };
    }

    pub fn map_iomem(&mut self, io_mem: IoMem, prop: PageProperty, len: usize, offset: usize) {
        assert_eq!(len % PAGE_SIZE, 0);
        assert_eq!(offset % PAGE_SIZE, 0);

        let io_mem_paddr = io_mem.paddr();
        let io_mem_size = io_mem.size();
        assert_eq!(io_mem_paddr % PAGE_SIZE, 0);
        assert_eq!(io_mem_size % PAGE_SIZE, 0);

        if offset >= io_mem_size {
            return;
        }

        let paddr_begin = io_mem_paddr + offset;
        let paddr_end = if io_mem_size - offset < len {
            io_mem_paddr + io_mem_size
        } else {
            io_mem_paddr + len + offset
        };

        for current_paddr in (paddr_begin..paddr_end).step_by(PAGE_SIZE) {
            unsafe {
                self.pt_cursor
                    .map(VmItem::new_untracked_io(current_paddr, prop))
            };
        }

        fn io_mem_contains(parent: &IoMem, child: &IoMem) -> bool {
            parent.paddr() <= child.paddr() && child.end_paddr() <= parent.end_paddr()
        }

        let mut iomems = self.vmspace.iomems.lock();
        if !iomems.iter().any(|iomem| io_mem_contains(iomem, &io_mem)) {
            iomems.retain(|iomem| !io_mem_contains(&io_mem, iomem));
            iomems.push(io_mem);
        }
    }

    pub fn find_iomem_by_paddr(&self, paddr: Paddr) -> Option<(IoMem, usize)> {
        self.vmspace.find_iomem_by_paddr(paddr)
    }

    pub fn unmap(&mut self, len: usize) -> usize {
        let end_va = self.virt_addr() + len;
        let mut num_unmapped: usize = 0;
        loop {
            let Some(frag) = (unsafe { self.pt_cursor.take_next(end_va - self.virt_addr()) })
            else {
                break;
            };

            match frag {
                PageTableFrag::Mapped { va, item, .. } => {
                    let (item, panic_guard) = unsafe { RcuDrop::into_inner(item) };

                    match item {
                        VmItem {
                            mapped_item: MappedItem::TrackedFrame(old_frame),
                            ..
                        } => {
                            num_unmapped += 1;

                            let rcu_frame = RcuDrop::new(old_frame);
                            panic_guard.forget();
                            let rcu_frame = Frame::rcu_from_unsized(rcu_frame);
                            self.flusher
                                .issue_tlb_flush_with(TlbFlushOp::for_single(va), rcu_frame);
                        }
                        VmItem {
                            mapped_item: MappedItem::UntrackedIoMem { .. },
                            ..
                        } => {
                            panic_guard.forget();

                            self.flusher.issue_tlb_flush(TlbFlushOp::for_single(va));
                        }
                    }
                }
                PageTableFrag::StrayPageTable {
                    pt,
                    va,
                    len,
                    num_frames,
                } => {
                    num_unmapped += num_frames;

                    self.flusher.issue_tlb_flush_with(
                        TlbFlushOp::for_range(va..va + len),
                        Frame::rcu_from_unsized(pt),
                    );
                }
            }
        }

        self.flusher.dispatch_tlb_flush();

        num_unmapped
    }

    pub fn protect_next(
        &mut self,
        len: usize,
        mut op: impl FnMut(&mut PageFlags, &mut CachePolicy),
    ) -> Option<Range<Vaddr>> {
        unsafe {
            self.pt_cursor.protect_next(len, &mut |prop| {
                op(&mut prop.flags, &mut prop.cache);
            })
        }
    }
}

cpu_local_cell! {
    static ACTIVATED_VM_SPACE: CpuLocalPtr<VmSpace> = CpuLocalPtr::null();
}

#[cfg(ktest)]
pub(super) fn get_activated_vm_space() -> *const VmSpace {
    ACTIVATED_VM_SPACE.load().as_ptr()
}

pub enum VmQueriedItem<'a> {
    MappedRam {
        frame: FrameRef<'a, dyn AnyUFrameMeta>,
        prop: PageProperty,
    },
    MappedIoMem {
        paddr: Paddr,
        prop: PageProperty,
    },
}

impl VmQueriedItem<'_> {
    pub fn prop(&self) -> &PageProperty {
        match self {
            Self::MappedRam { prop, .. } => prop,
            Self::MappedIoMem { prop, .. } => prop,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct VmItem {
    prop: PageProperty,
    mapped_item: MappedItem,
}

#[derive(Debug)]
pub(crate) struct VmItemRef<'a> {
    prop: PageProperty,
    mapped_item: MappedItemRef<'a>,
}

#[derive(Clone, Debug, PartialEq)]
enum MappedItem {
    TrackedFrame(UFrame),
    UntrackedIoMem { paddr: Paddr, level: PagingLevel },
}

#[derive(Debug)]
enum MappedItemRef<'a> {
    TrackedFrame(FrameRef<'a, dyn AnyUFrameMeta>),
    UntrackedIoMem { paddr: Paddr, level: PagingLevel },
}

impl VmItem {
    pub(super) fn new_tracked(frame: UFrame, prop: PageProperty) -> Self {
        Self {
            prop,
            mapped_item: MappedItem::TrackedFrame(frame),
        }
    }

    fn new_untracked_io(paddr: Paddr, prop: PageProperty) -> Self {
        Self {
            prop,
            mapped_item: MappedItem::UntrackedIoMem { paddr, level: 1 },
        }
    }
}

impl<'a> From<VmItemRef<'a>> for VmQueriedItem<'a> {
    fn from(item: VmItemRef<'a>) -> Self {
        match item.mapped_item {
            MappedItemRef::TrackedFrame(frame) => VmQueriedItem::MappedRam {
                frame,
                prop: item.prop,
            },
            MappedItemRef::UntrackedIoMem { paddr, level } => {
                debug_assert_eq!(level, 1);
                VmQueriedItem::MappedIoMem {
                    paddr,
                    prop: item.prop,
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct UserPtConfig {}

unsafe impl PageTableConfig for UserPtConfig {
    const TOP_LEVEL_INDEX_RANGE: Range<usize> = 0..256;

    type E = PageTableEntry;
    type C = PagingConsts;

    type Item = VmItem;
    type ItemRef<'a> = VmItemRef<'a>;

    fn item_raw_info(item: &Self::Item) -> (Paddr, PagingLevel, PageProperty) {
        match &item.mapped_item {
            MappedItem::TrackedFrame(frame) => {
                let mut prop = item.prop;
                prop.priv_flags -= PrivilegedPageFlags::AVAIL1;
                let level = frame.map_level();
                let paddr = frame.paddr();
                (paddr, level, prop)
            }
            MappedItem::UntrackedIoMem { paddr, level } => {
                let mut prop = item.prop;
                prop.priv_flags |= PrivilegedPageFlags::AVAIL1;
                (*paddr, *level, prop)
            }
        }
    }

    unsafe fn item_from_raw(paddr: Paddr, level: PagingLevel, prop: PageProperty) -> Self::Item {
        debug_assert_eq!(level, 1);
        if prop.priv_flags.contains(PrivilegedPageFlags::AVAIL1) {
            VmItem::new_untracked_io(paddr, prop)
        } else {
            let frame = unsafe { Frame::<dyn AnyUFrameMeta>::from_raw(paddr) };
            VmItem::new_tracked(frame, prop)
        }
    }

    unsafe fn item_ref_from_raw<'a>(
        paddr: Paddr,
        level: PagingLevel,
        prop: PageProperty,
    ) -> Self::ItemRef<'a> {
        debug_assert_eq!(level, 1);
        if prop.priv_flags.contains(PrivilegedPageFlags::AVAIL1) {
            VmItemRef {
                prop,
                mapped_item: MappedItemRef::UntrackedIoMem { paddr, level },
            }
        } else {
            let frame_ref = unsafe { FrameRef::<dyn AnyUFrameMeta>::borrow_paddr(paddr) };
            VmItemRef {
                prop,
                mapped_item: MappedItemRef::TrackedFrame(frame_ref),
            }
        }
    }
}
