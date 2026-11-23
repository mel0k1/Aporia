use core::ops::Range;

use self::allocator::kvirt_area_allocator;
use super::{KERNEL_PAGE_TABLE, KernelPtConfig, MappedItem};
use crate::{
    irq,
    mm::{
        HasSize, PAGE_SIZE, Paddr, Split, Vaddr,
        frame::{Frame, meta::AnyFrameMeta},
        page_prop::PageProperty,
        page_table::{largest_pages, max_page_level},
    },
};

mod allocator {
    use crate::{
        irq::DisabledLocalIrqGuard, mm::kspace::VMALLOC_VADDR_RANGE,
        util::range_alloc::RangeAllocator,
    };

    static KVIRT_AREA_ALLOCATOR: RangeAllocator = RangeAllocator::new(VMALLOC_VADDR_RANGE);

    pub(super) fn kvirt_area_allocator(_guard: &DisabledLocalIrqGuard) -> &RangeAllocator {
        &KVIRT_AREA_ALLOCATOR
    }
}

#[derive(Debug)]
pub(crate) struct KVirtArea {
    range: Range<Vaddr>,
}

impl HasSize for KVirtArea {
    fn size(&self) -> usize {
        self.range.len()
    }
}

impl Split for KVirtArea {
    fn split(self, offset: usize) -> (Self, Self) {
        assert!(offset.is_multiple_of(PAGE_SIZE));
        assert!(0 < offset && offset < self.size());

        let old = core::mem::ManuallyDrop::new(self);

        let left_range = old.start()..old.start() + offset;
        let right_range = old.start() + offset..old.end();
        (
            KVirtArea { range: left_range },
            KVirtArea { range: right_range },
        )
    }
}

impl KVirtArea {
    pub(crate) fn start(&self) -> Vaddr {
        self.range.start
    }

    pub(crate) fn end(&self) -> Vaddr {
        self.range.end
    }

    pub(crate) fn range(&self) -> Range<Vaddr> {
        self.range.start..self.range.end
    }

    #[cfg(ktest)]
    pub(crate) fn query<'a, G: crate::task::atomic_mode::AsAtomicModeGuard>(
        &'a self,
        guard: &'a G,
        addr: Vaddr,
    ) -> Option<super::MappedItemRef<'a>> {
        use align_ext::AlignExt;

        assert!(self.start() <= addr && self.end() >= addr);

        let start = addr.align_down(PAGE_SIZE);
        let vaddr = start..start + PAGE_SIZE;

        let page_table = KERNEL_PAGE_TABLE.get().unwrap();
        let mut cursor = page_table.cursor(guard, &vaddr).unwrap();

        cursor.query().unwrap().1
    }

    pub(crate) fn map_frames<T: AnyFrameMeta + ?Sized>(
        area_size: usize,
        map_offset: usize,
        frames: impl Iterator<Item = Frame<T>>,
        prop: PageProperty,
    ) -> Self {
        assert!(area_size.is_multiple_of(PAGE_SIZE));
        assert!(map_offset.is_multiple_of(PAGE_SIZE));

        let irq_guard = irq::disable_local();

        let range = kvirt_area_allocator(&irq_guard).alloc(area_size).unwrap();
        let cursor_range = range.start + map_offset..range.end;

        let page_table = KERNEL_PAGE_TABLE.get().unwrap();
        let mut cursor = page_table.cursor_mut(&irq_guard, &cursor_range).unwrap();

        for frame in frames.into_iter() {
            unsafe { cursor.map(MappedItem::Tracked(Frame::from_unsized(frame), prop)) };
        }

        Self { range }
    }

    pub(crate) unsafe fn map_untracked_frames(
        area_size: usize,
        map_offset: usize,
        pa_range: Range<Paddr>,
        prop: PageProperty,
    ) -> Self {
        assert!(pa_range.start.is_multiple_of(PAGE_SIZE));
        assert!(pa_range.end.is_multiple_of(PAGE_SIZE));
        assert!(area_size.is_multiple_of(PAGE_SIZE));
        assert!(map_offset.is_multiple_of(PAGE_SIZE));
        assert!(map_offset + pa_range.len() <= area_size);

        let irq_guard = irq::disable_local();

        let range = kvirt_area_allocator(&irq_guard).alloc(area_size).unwrap();

        if !pa_range.is_empty() {
            let len = pa_range.len();
            let va_range = range.start + map_offset..range.start + map_offset + len;

            let page_table = KERNEL_PAGE_TABLE.get().unwrap();
            let min_level = max_page_level::<KernelPtConfig>(len);
            let mut cursor = page_table
                .cursor_mut_with_min_level(&irq_guard, &va_range, min_level)
                .unwrap();

            for (pa, level) in largest_pages::<KernelPtConfig>(va_range.start, pa_range.start, len)
            {
                unsafe { cursor.map(MappedItem::Untracked(pa, level, prop)) };
            }
        }

        Self { range }
    }
}

impl Drop for KVirtArea {
    fn drop(&mut self) {
        let irq_guard = irq::disable_local();

        let page_table = KERNEL_PAGE_TABLE.get().unwrap();
        let range = self.start()..self.end();
        let mut cursor = page_table.cursor_mut(&irq_guard, &range).unwrap();
        loop {
            let Some(frag) = (unsafe { cursor.take_next(self.end() - cursor.virt_addr()) }) else {
                break;
            };
            drop(frag);
        }

        kvirt_area_allocator(&irq_guard).free(range);
    }
}
