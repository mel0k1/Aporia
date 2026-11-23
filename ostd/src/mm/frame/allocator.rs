use core::{alloc::Layout, ops::Range};

use align_ext::AlignExt;

use super::{Frame, meta::AnyFrameMeta, segment::Segment};
use crate::{
    boot::memory_region::MemoryRegionType,
    error::Error,
    impl_frame_meta_for,
    mm::{PAGE_SIZE, paddr_to_vaddr},
    prelude::*,
    util::ops::range_difference,
};

pub struct FrameAllocOptions {
    zeroed: bool,
}

impl Default for FrameAllocOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameAllocOptions {
    pub fn new() -> Self {
        Self { zeroed: true }
    }

    pub fn zeroed(&mut self, zeroed: bool) -> &mut Self {
        self.zeroed = zeroed;
        self
    }

    pub fn alloc_frame(&self) -> Result<Frame<()>> {
        self.alloc_frame_with(())
    }

    pub fn alloc_frame_with<M: AnyFrameMeta>(&self, metadata: M) -> Result<Frame<M>> {
        let single_layout = Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).unwrap();
        let frame = get_global_frame_allocator()
            .alloc(single_layout)
            .map(|paddr| Frame::from_unused(paddr, metadata).unwrap())
            .ok_or(Error::NoMemory)?;

        if self.zeroed {
            let addr = paddr_to_vaddr(frame.paddr()) as *mut u8;
            unsafe { core::ptr::write_bytes(addr, 0, PAGE_SIZE) }
        }

        Ok(frame)
    }

    pub fn alloc_segment(&self, nframes: usize) -> Result<Segment<()>> {
        self.alloc_segment_with(nframes, |_| ())
    }

    pub fn alloc_segment_with<M: AnyFrameMeta, F>(
        &self,
        nframes: usize,
        metadata_fn: F,
    ) -> Result<Segment<M>>
    where
        F: FnMut(Paddr) -> M,
    {
        if nframes == 0 {
            return Err(Error::InvalidArgs);
        }
        let layout = Layout::from_size_align(nframes * PAGE_SIZE, PAGE_SIZE).unwrap();
        let segment = get_global_frame_allocator()
            .alloc(layout)
            .map(|start| {
                Segment::from_unused(start..start + nframes * PAGE_SIZE, metadata_fn).unwrap()
            })
            .ok_or(Error::NoMemory)?;

        if self.zeroed {
            let addr = paddr_to_vaddr(segment.paddr()) as *mut u8;
            unsafe { core::ptr::write_bytes(addr, 0, nframes * PAGE_SIZE) }
        }

        Ok(segment)
    }
}

#[cfg(ktest)]
#[ktest(expect_redundant_test_prefix)]
fn test_alloc_dealloc() {
    let single_options = FrameAllocOptions::new();
    let mut contiguous_options = FrameAllocOptions::new();
    contiguous_options.zeroed(false);
    let mut remember_vec = Vec::new();
    for _ in 0..10 {
        for i in 0..10 {
            let single_frame = single_options.alloc_frame().unwrap();
            if i % 3 == 0 {
                remember_vec.push(single_frame);
            }
        }
        let contiguous_segment = contiguous_options.alloc_segment(10).unwrap();
        drop(contiguous_segment);
        remember_vec.pop();
    }
}

pub trait GlobalFrameAllocator: Sync {
    fn alloc(&self, layout: Layout) -> Option<Paddr>;

    fn dealloc(&self, addr: Paddr, size: usize);

    fn add_free_memory(&self, addr: Paddr, size: usize);
}

unsafe extern "Rust" {
    static __GLOBAL_FRAME_ALLOCATOR_REF: &'static dyn GlobalFrameAllocator;
}

pub(super) fn get_global_frame_allocator() -> &'static dyn GlobalFrameAllocator {
    unsafe { __GLOBAL_FRAME_ALLOCATOR_REF }
}

pub(crate) unsafe fn init() {
    let early_allocator = EARLY_ALLOCATOR.lock().take().unwrap();
    let early_allocated_ranges = early_allocator.allocated_regions();

    #[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
    let unaccepted_range = crate::if_tdx_enabled!({
        super::unaccepted::init_unaccepted_memory_state(&early_allocated_ranges)
    } else {
        0..0
    });
    #[cfg(not(all(target_arch = "x86_64", feature = "cvm_guest")))]
    let unaccepted_range: Range<Paddr> = 0..0;

    let regions = &crate::boot::EARLY_INFO.get().unwrap().memory_regions;
    let [range_1, range_2] = &early_allocated_ranges;

    for region in regions
        .iter()
        .filter(|region| region.typ() == MemoryRegionType::Usable)
    {
        debug_assert!(region.base().is_multiple_of(PAGE_SIZE));
        debug_assert!(region.len().is_multiple_of(PAGE_SIZE));

        let region_range = region.base()..region.end();
        for free_range in range_difference(&region_range, range_1)
            .flat_map(|range| range_difference(&range, range_2))
            .flat_map(|range| range_difference(&range, &unaccepted_range))
        {
            crate::info!("Adding free frames to the allocator: {:x?}", free_range);
            get_global_frame_allocator().add_free_memory(free_range.start, free_range.len());
        }
    }
}

pub(super) struct EarlyFrameAllocator {
    under_4g_range: Range<Paddr>,
    under_4g_end: Paddr,

    max_range: Range<Paddr>,
    max_end: Paddr,
}

pub(super) static EARLY_ALLOCATOR: spin::Mutex<Option<EarlyFrameAllocator>> =
    spin::Mutex::new(None);

impl EarlyFrameAllocator {
    pub(crate) fn new() -> Self {
        let regions = &crate::boot::EARLY_INFO.get().unwrap().memory_regions;

        let mut under_4g_range = 0..0;
        let mut max_range = 0..0;
        for region in regions.iter() {
            if region.typ() != MemoryRegionType::Usable {
                continue;
            }
            const PADDR4G: Paddr = 0x1_0000_0000;
            if region.base() < PADDR4G {
                let range = region.base()..region.end().min(PADDR4G);
                if range.len() > under_4g_range.len() {
                    under_4g_range = range;
                }
            }
            if region.end() >= PADDR4G {
                let range = region.base().max(PADDR4G)..region.end();
                if range.len() > max_range.len() {
                    max_range = range;
                }
            }
        }

        crate::debug!(
            "Early frame allocator (below 4G) at: {:#x?}",
            under_4g_range
        );
        if !max_range.is_empty() {
            crate::debug!("Early frame allocator (above 4G) at: {:#x?}", max_range);
        }

        Self {
            under_4g_range: under_4g_range.clone(),
            under_4g_end: under_4g_range.start,
            max_range: max_range.clone(),
            max_end: max_range.start,
        }
    }

    pub(super) fn alloc(&mut self, layout: Layout) -> Option<Paddr> {
        let size = layout.size().align_up(PAGE_SIZE);
        let align = layout.align().max(PAGE_SIZE);

        for (tail, end) in [
            (&mut self.under_4g_end, self.under_4g_range.end),
            (&mut self.max_end, self.max_range.end),
        ] {
            let allocated = tail.align_up(align);
            if let Some(allocated_end) = allocated.checked_add(size)
                && allocated_end <= end
            {
                *tail = allocated_end;
                return Some(allocated);
            }
        }

        None
    }

    pub(super) fn allocated_regions(&self) -> [Range<Paddr>; 2] {
        [
            self.under_4g_range.start..self.under_4g_end,
            self.max_range.start..self.max_end,
        ]
    }
}

#[derive(Debug)]
pub(crate) struct EarlyAllocatedFrameMeta;

impl_frame_meta_for!(EarlyAllocatedFrameMeta);

pub(crate) fn early_alloc(layout: Layout) -> Option<Paddr> {
    let mut early_allocator = EARLY_ALLOCATOR.lock();
    let paddr = early_allocator.as_mut().unwrap().alloc(layout)?;

    #[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
    crate::if_tdx_enabled!({
        unsafe { super::unaccepted::accept_early_allocated_range(paddr, layout.size()) };
    });

    Some(paddr)
}

pub(crate) unsafe fn init_early_allocator() {
    let mut early_allocator = EARLY_ALLOCATOR.lock();
    *early_allocator = Some(EarlyFrameAllocator::new());
}
