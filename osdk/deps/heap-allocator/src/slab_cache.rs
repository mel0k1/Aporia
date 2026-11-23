use core::alloc::AllocError;

use ostd::mm::{
    PAGE_SIZE, Paddr,
    frame::linked_list::LinkedList,
    heap::{HeapSlot, Slab, SlabMeta},
};

const EXPECTED_EMPTY_SLABS: usize = 4;
const MAX_EMPTY_SLABS: usize = 16;

pub(crate) struct SlabCache<const SLOT_SIZE: usize> {
    empty: LinkedList<SlabMeta<SLOT_SIZE>>,
    partial: LinkedList<SlabMeta<SLOT_SIZE>>,
    full: LinkedList<SlabMeta<SLOT_SIZE>>,
}

impl<const SLOT_SIZE: usize> SlabCache<SLOT_SIZE> {
    pub(crate) const fn new() -> Self {
        Self {
            empty: LinkedList::new(),
            partial: LinkedList::new(),
            full: LinkedList::new(),
        }
    }

    pub(crate) fn alloc(&mut self) -> Result<HeapSlot, AllocError> {
        if !self.partial.is_empty() {
            let mut cursor = self.partial.cursor_back_mut();
            let current = cursor.current_meta().unwrap();
            let allocated = current.alloc().unwrap();
            if current.nr_allocated() == current.capacity() {
                self.full.push_front(cursor.take_current().unwrap());
            }
            return Ok(allocated);
        }

        if !self.empty.is_empty() {
            let mut slab = self.empty.pop_front().unwrap();
            let allocated = slab.meta_mut().alloc().unwrap();
            self.add_slab(slab);
            return Ok(allocated);
        }

        let Some(mut allocated_empty) = Self::alloc_slab() else {
            log::error!("Failed to allocate a new slab");
            return Err(AllocError);
        };
        let allocated = allocated_empty.meta_mut().alloc().unwrap();
        self.add_slab(allocated_empty);

        for _ in 0..EXPECTED_EMPTY_SLABS {
            if let Some(allocated_empty) = Self::alloc_slab() {
                self.empty.push_front(allocated_empty);
            } else {
                break;
            }
        }

        Ok(allocated)
    }

    pub(crate) fn dealloc(&mut self, slot: HeapSlot) -> Result<(), AllocError> {
        let which = which_slab(&slot).ok_or_else(|| {
            log::error!("Can't find the slab for the slot");
            AllocError
        })?;

        let mut extracted_slab = None;

        if self.partial.contains(which) {
            extracted_slab = self.partial.cursor_mut_at(which).unwrap().take_current();
        } else if self.full.contains(which) {
            extracted_slab = self.full.cursor_mut_at(which).unwrap().take_current();
        }

        let mut slab = extracted_slab.ok_or_else(|| {
            log::error!("Deallocating a slot that is not allocated from the cache");
            AllocError
        })?;

        slab.dealloc(slot)?;

        self.add_slab(slab);

        if self.empty.size() > MAX_EMPTY_SLABS {
            while self.empty.size() > EXPECTED_EMPTY_SLABS {
                self.empty.pop_front();
                crate::TOTAL_SLAB_ALLOCATED.sub(PAGE_SIZE);
            }
        }

        Ok(())
    }

    fn alloc_slab() -> Option<Slab<SLOT_SIZE>> {
        let slab = Slab::new().ok()?;
        crate::TOTAL_SLAB_ALLOCATED.add(PAGE_SIZE);
        Some(slab)
    }

    fn add_slab(&mut self, slab: Slab<SLOT_SIZE>) {
        if slab.meta().nr_allocated() == slab.meta().capacity() {
            self.full.push_front(slab);
        } else if slab.meta().nr_allocated() > 0 {
            self.partial.push_back(slab);
        } else {
            self.empty.push_front(slab);
        }
    }
}

impl<const SLOT_SIZE: usize> Drop for SlabCache<SLOT_SIZE> {
    fn drop(&mut self) {
        for list in [&mut self.empty, &mut self.partial, &mut self.full] {
            while list.pop_front().is_some() {
                crate::TOTAL_SLAB_ALLOCATED.sub(PAGE_SIZE);
            }
        }
    }
}

fn which_slab(slot: &HeapSlot) -> Option<Paddr> {
    if slot.size() > PAGE_SIZE {
        return None;
    }

    let frame_paddr = slot.paddr() / PAGE_SIZE * PAGE_SIZE;
    Some(frame_paddr)
}
