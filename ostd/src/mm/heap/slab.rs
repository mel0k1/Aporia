use core::{alloc::AllocError, ptr::NonNull};

use super::{slot::HeapSlot, slot_list::SlabSlotList};
use crate::mm::{
    FrameAllocOptions, HasPaddr, HasSize, PAGE_SIZE, UniqueFrame,
    frame::{linked_list::Link, meta::AnyFrameMeta},
    paddr_to_vaddr,
};

pub type Slab<const SLOT_SIZE: usize> = UniqueFrame<Link<SlabMeta<SLOT_SIZE>>>;

#[derive(Debug)]
pub struct SlabMeta<const SLOT_SIZE: usize> {
    free_list: SlabSlotList<SLOT_SIZE>,

    nr_allocated: u16,
}

unsafe impl<const SLOT_SIZE: usize> Send for SlabMeta<SLOT_SIZE> {}
unsafe impl<const SLOT_SIZE: usize> Sync for SlabMeta<SLOT_SIZE> {}

unsafe impl<const SLOT_SIZE: usize> AnyFrameMeta for SlabMeta<SLOT_SIZE> {
    fn on_drop(&mut self, _reader: &mut crate::mm::VmReader<crate::mm::Infallible>) {
        if self.nr_allocated != 0 {
            panic!("{} slots allocated when dropping a slab", self.nr_allocated);
        }
    }

    fn is_untyped(&self) -> bool {
        false
    }
}

impl<const SLOT_SIZE: usize> SlabMeta<SLOT_SIZE> {
    pub const fn capacity(&self) -> u16 {
        (PAGE_SIZE / SLOT_SIZE) as u16
    }

    pub fn nr_allocated(&self) -> u16 {
        self.nr_allocated
    }

    pub fn alloc(&mut self) -> Result<HeapSlot, AllocError> {
        let Some(allocated) = self.free_list.pop() else {
            crate::error!("Allocating a slot from a full slab");
            return Err(AllocError);
        };
        self.nr_allocated += 1;
        Ok(allocated)
    }
}

impl<const SLOT_SIZE: usize> Slab<SLOT_SIZE> {
    pub fn new() -> crate::prelude::Result<Self> {
        const { assert!(SLOT_SIZE <= PAGE_SIZE) };
        const { assert!(SLOT_SIZE >= size_of::<usize>()) };
        const { assert!(PAGE_SIZE / SLOT_SIZE <= u16::MAX as usize) };

        let mut slab: Slab<SLOT_SIZE> = FrameAllocOptions::new()
            .zeroed(false)
            .alloc_frame_with(Link::new(SlabMeta::<SLOT_SIZE> {
                free_list: SlabSlotList::new(),
                nr_allocated: 0,
            }))?
            .try_into()
            .unwrap();

        let head_paddr = slab.paddr();
        let head_vaddr = paddr_to_vaddr(head_paddr);

        for slot_offset in (0..PAGE_SIZE).step_by(SLOT_SIZE) {
            let slot_ptr = unsafe { NonNull::new_unchecked((head_vaddr + slot_offset) as *mut u8) };
            slab.meta_mut()
                .free_list
                .push(unsafe { HeapSlot::new(slot_ptr, super::SlotInfo::SlabSlot(SLOT_SIZE)) });
        }

        Ok(slab)
    }

    pub fn dealloc(&mut self, slot: HeapSlot) -> Result<(), AllocError> {
        if !(self.paddr()..self.paddr() + self.size()).contains(&slot.paddr()) {
            crate::error!("Deallocating a slot to a slab that does not own the slot");
            return Err(AllocError);
        }
        debug_assert_eq!(slot.size(), SLOT_SIZE);
        self.meta_mut().free_list.push(slot);
        self.meta_mut().nr_allocated -= 1;

        Ok(())
    }
}
