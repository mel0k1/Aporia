use core::{mem::ManuallyDrop, ops::Range};

use super::Gpaddr;
use crate::{
    arch::vm::memory::{ArchGuestPhysMemSpace, PageTableEntry, PagingConsts},
    mm::{
        AnyUFrameMeta, Frame, PageFlags, PageProperty, PagingLevel, UFrame,
        frame::FrameRef,
        page_table::{self, PageTable, PageTableConfig, PageTableFrag},
    },
    prelude::*,
    smp::PendingIpis,
    sync::RcuDrop,
    task::atomic_mode::AsAtomicModeGuard,
};

pub struct GuestPhysMemSpace {
    pt: PageTable<GuestPtConfig>,
    arch: ArchGuestPhysMemSpace,
}

impl GuestPhysMemSpace {
    pub fn new() -> Result<Self> {
        let arch = ArchGuestPhysMemSpace::new()?;
        let pt = PageTable::<GuestPtConfig>::empty();

        arch.invalidate(&[]).wait();

        Ok(Self { pt, arch })
    }

    pub fn cursor<'a, G: AsAtomicModeGuard>(
        &'a self,
        guard: &'a G,
        gpa: &Range<Gpaddr>,
    ) -> Result<Cursor<'a>> {
        Ok(Cursor(self.pt.cursor(guard, gpa)?))
    }

    pub fn cursor_mut<'a, G: AsAtomicModeGuard>(
        &'a self,
        guard: &'a G,
        gpa: &Range<Gpaddr>,
    ) -> Result<CursorMut<'a>> {
        Ok(CursorMut {
            pt_cursor: self.pt.cursor_mut(guard, gpa)?,
            arch: &self.arch,
            pending_ipis: PendingIpis::new_empty(),
        })
    }

    pub(crate) fn root_paddr(&self) -> u64 {
        self.pt.root_paddr() as u64
    }
}

pub type QueriedItem<'a> = (FrameRef<'a, dyn AnyUFrameMeta>, PageProperty);

pub struct Cursor<'a>(page_table::Cursor<'a, GuestPtConfig>);

impl Cursor<'_> {
    pub fn query(&mut self) -> Result<(Range<Gpaddr>, Option<QueriedItem<'_>>)> {
        Ok(self.0.query()?)
    }

    pub fn find_next(&mut self, len: usize) -> Option<Gpaddr> {
        self.0.find_next(len)
    }

    pub fn jump(&mut self, gpa: Gpaddr) -> Result<()> {
        self.0.jump(gpa)?;
        Ok(())
    }

    pub fn gpa(&self) -> Gpaddr {
        self.0.virt_addr()
    }
}

pub struct CursorMut<'a> {
    pt_cursor: page_table::CursorMut<'a, GuestPtConfig>,
    arch: &'a ArchGuestPhysMemSpace,
    pending_ipis: PendingIpis,
}

impl<'a> CursorMut<'a> {
    pub fn query(&mut self) -> Result<(Range<Gpaddr>, Option<QueriedItem<'_>>)> {
        Ok(self.pt_cursor.query()?)
    }

    pub fn find_next(&mut self, len: usize) -> Option<Gpaddr> {
        self.pt_cursor.find_next(len)
    }

    pub fn jump(&mut self, gpa: Gpaddr) -> Result<()> {
        self.pt_cursor.jump(gpa)?;
        Ok(())
    }

    pub fn gpa(&self) -> Gpaddr {
        self.pt_cursor.virt_addr()
    }

    pub fn map(&mut self, frame: UFrame, prop: PageProperty) {
        let item: GuestPtItem = (frame, prop);

        unsafe { self.pt_cursor.map(item) };
    }

    pub fn protect_next(
        &mut self,
        len: usize,
        op: &mut impl FnMut(&mut PageFlags),
    ) -> Option<Range<Gpaddr>> {
        let range = unsafe {
            self.pt_cursor
                .protect_next(len, &mut |prop| op(&mut prop.flags))
        }?;
        self.pending_ipis.extend(&self.arch.invalidate(&[]));
        Some(range)
    }

    pub fn unmap(&mut self, len: usize) -> usize {
        let end_gpa = self.gpa() + len;
        let mut num_unmapped: usize = 0;
        let mut frames = ManuallyDrop::new(Vec::new());
        loop {
            frames.reserve(1);
            let Some(frag) = (unsafe { self.pt_cursor.take_next(end_gpa - self.gpa()) }) else {
                break;
            };

            match frag {
                PageTableFrag::Mapped { item, .. } => {
                    let ((frame, _), panic_guard) = unsafe { RcuDrop::into_inner(item) };
                    frames.push(Frame::rcu_from_unsized(RcuDrop::new(frame)));
                    panic_guard.forget();
                    num_unmapped += 1;
                }
                PageTableFrag::StrayPageTable { pt, num_frames, .. } => {
                    frames.push(Frame::rcu_from_unsized(pt));
                    num_unmapped += num_frames;
                }
            }
        }

        if !frames.is_empty() {
            self.pending_ipis.extend(&self.arch.invalidate(&frames));
        }
        drop(ManuallyDrop::into_inner(frames));
        num_unmapped
    }

    pub fn sync_tlb_flush(&mut self) {
        self.pending_ipis.wait();
        self.pending_ipis = PendingIpis::new_empty();
    }
}

#[derive(Clone, Debug)]
pub(crate) struct GuestPtConfig {}

unsafe impl PageTableConfig for GuestPtConfig {
    const TOP_LEVEL_INDEX_RANGE: Range<usize> = 0..512;

    type E = PageTableEntry;
    type C = PagingConsts;

    type Item = GuestPtItem;
    type ItemRef<'a> = GuestPtItemRef<'a>;

    fn item_raw_info(item: &Self::Item) -> (Paddr, PagingLevel, PageProperty) {
        let (frame, prop) = item;
        (frame.paddr(), frame.map_level(), *prop)
    }

    unsafe fn item_from_raw(paddr: Paddr, level: PagingLevel, prop: PageProperty) -> Self::Item {
        debug_assert_eq!(level, 1);
        let frame = unsafe { Frame::<dyn AnyUFrameMeta>::from_raw(paddr) };
        (frame, prop)
    }

    unsafe fn item_ref_from_raw<'a>(
        paddr: Paddr,
        level: PagingLevel,
        prop: PageProperty,
    ) -> Self::ItemRef<'a> {
        debug_assert_eq!(level, 1);
        let frame = unsafe { FrameRef::<dyn AnyUFrameMeta>::borrow_paddr(paddr) };
        (frame, prop)
    }
}

pub(crate) type GuestPtItem = (UFrame, PageProperty);
pub(crate) type GuestPtItemRef<'a> = (FrameRef<'a, dyn AnyUFrameMeta>, PageProperty);

#[cfg(ktest)]
mod test {
    use super::*;
    use crate::{
        Error,
        mm::{CachePolicy, FrameAllocOptions, PAGE_SIZE},
        task::disable_preempt,
    };

    #[ktest]
    fn guest_mapping_lifecycle() {
        let space = match GuestPhysMemSpace::new() {
            Ok(space) => space,
            Err(Error::NotEnoughResources | Error::AccessDenied) => {
                crate::early_print!(" [skipped: VMX/EPT invalidation unavailable]");
                return;
            }
            Err(err) => panic!("failed to create guest physical memory: {:?}", err),
        };

        let first = FrameAllocOptions::new().alloc_frame().unwrap();
        let second = FrameAllocOptions::new().alloc_frame().unwrap();
        let first_paddr = first.paddr();
        let second_paddr = second.paddr();
        let prop = PageProperty::new_guest(PageFlags::RWX, CachePolicy::Writeback);
        let range = PAGE_SIZE..4 * PAGE_SIZE;

        let guard = disable_preempt();
        let mut cursor = space.cursor_mut(&guard, &range).unwrap();

        cursor.map(first.into(), prop);
        cursor.map(second.into(), prop);
        assert_eq!(cursor.gpa(), 3 * PAGE_SIZE);

        cursor.jump(2 * PAGE_SIZE).unwrap();
        let (queried_range, item) = cursor.query().unwrap();
        assert_eq!(queried_range, 2 * PAGE_SIZE..3 * PAGE_SIZE);
        assert_eq!(
            item.map(|(frame, prop)| (frame.paddr(), prop)),
            Some((second_paddr, prop))
        );
        cursor.jump(PAGE_SIZE).unwrap();
        let (queried_range, item) = cursor.query().unwrap();
        assert_eq!(queried_range, PAGE_SIZE..2 * PAGE_SIZE);
        assert_eq!(
            item.map(|(frame, prop)| (frame.paddr(), prop)),
            Some((first_paddr, prop))
        );

        assert_eq!(
            cursor.protect_next(PAGE_SIZE, &mut |flags| *flags = PageFlags::RX),
            Some(PAGE_SIZE..2 * PAGE_SIZE)
        );
        cursor.jump(PAGE_SIZE).unwrap();
        assert_eq!(cursor.query().unwrap().1.unwrap().1.flags, PageFlags::RX);

        assert_eq!(cursor.unmap(2 * PAGE_SIZE), 2);
        cursor.jump(PAGE_SIZE).unwrap();
        assert!(cursor.find_next(3 * PAGE_SIZE).is_none());
    }
}
