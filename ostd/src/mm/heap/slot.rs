use core::{alloc::AllocError, ptr::NonNull};

use crate::{
    impl_frame_meta_for,
    mm::{
        FrameAllocOptions, PAGE_SIZE, Paddr, Segment, Vaddr, kspace::LINEAR_MAPPING_BASE_VADDR,
        paddr_to_vaddr,
    },
};

pub struct HeapSlot {
    addr: NonNull<u8>,
    info: SlotInfo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlotInfo {
    SlabSlot(usize),
    LargeSlot(usize),
}

impl SlotInfo {
    pub fn size(&self) -> usize {
        match self {
            Self::SlabSlot(size) => *size,
            Self::LargeSlot(size) => *size,
        }
    }
}

impl HeapSlot {
    pub(super) unsafe fn new(addr: NonNull<u8>, info: SlotInfo) -> Self {
        Self { addr, info }
    }

    pub fn alloc_large(size: usize) -> Result<Self, AllocError> {
        assert_eq!(size % PAGE_SIZE, 0);
        let nframes = size / PAGE_SIZE;
        let segment = FrameAllocOptions::new()
            .zeroed(false)
            .alloc_segment_with(nframes, |_| LargeAllocFrameMeta)
            .map_err(|_| {
                crate::error!("Failed to allocate a large slot");
                AllocError
            })?;

        let paddr_range = segment.into_raw();
        let vaddr = paddr_to_vaddr(paddr_range.start);

        Ok(Self {
            addr: NonNull::new(vaddr as *mut u8).unwrap(),
            info: SlotInfo::LargeSlot(size),
        })
    }

    pub fn dealloc_large(self) {
        let SlotInfo::LargeSlot(size) = self.info else {
            crate::error!(
                "Deallocating a large slot that was not allocated with `HeapSlot::alloc_large`"
            );
            crate::panic::abort();
        };

        debug_assert_eq!(size % PAGE_SIZE, 0);
        debug_assert_eq!(self.paddr() % PAGE_SIZE, 0);
        let range = self.paddr()..self.paddr() + size;

        drop(unsafe { Segment::<LargeAllocFrameMeta>::from_raw(range) });
    }

    pub fn paddr(&self) -> Paddr {
        self.addr.as_ptr() as Vaddr - LINEAR_MAPPING_BASE_VADDR
    }

    pub fn size(&self) -> usize {
        match self.info {
            SlotInfo::SlabSlot(size) => size,
            SlotInfo::LargeSlot(size) => size,
        }
    }

    pub fn info(&self) -> SlotInfo {
        self.info
    }

    pub fn as_ptr(&self) -> *mut u8 {
        self.addr.as_ptr()
    }
}

#[derive(Debug)]
pub(crate) struct LargeAllocFrameMeta;

impl_frame_meta_for!(LargeAllocFrameMeta);
