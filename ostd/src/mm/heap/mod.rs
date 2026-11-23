use core::{
    alloc::{AllocError, GlobalAlloc, Layout},
    ptr::NonNull,
};

use crate::mm::Vaddr;

mod slab;
mod slot;
mod slot_list;

pub use self::{
    slab::{Slab, SlabMeta},
    slot::{HeapSlot, SlotInfo},
    slot_list::SlabSlotList,
};

pub trait GlobalHeapAllocator: Sync {
    fn alloc(&self, layout: Layout) -> Result<HeapSlot, AllocError>;

    fn dealloc(&self, slot: HeapSlot) -> Result<(), AllocError>;
}

unsafe extern "Rust" {
    static __GLOBAL_HEAP_ALLOCATOR_REF: &'static dyn GlobalHeapAllocator;

    fn __global_heap_slot_info_from_layout(layout: Layout) -> Option<SlotInfo>;
}

fn get_global_heap_allocator() -> &'static dyn GlobalHeapAllocator {
    unsafe { __GLOBAL_HEAP_ALLOCATOR_REF }
}

fn slot_size_from_layout(layout: Layout) -> Option<SlotInfo> {
    unsafe { __global_heap_slot_info_from_layout(layout) }
}

macro_rules! abort_with_message {
    ($($arg:tt)*) => {
        crate::error!($($arg)*);
        crate::panic::abort();
    };
}

#[alloc_error_handler]
fn handle_alloc_error(layout: Layout) -> ! {
    abort_with_message!("Heap allocation error, layout = {:#x?}", layout);
}

#[global_allocator]
static HEAP_ALLOCATOR: AllocDispatch = AllocDispatch;

struct AllocDispatch;

unsafe impl GlobalAlloc for AllocDispatch {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let Some(required_slot) = slot_size_from_layout(layout) else {
            abort_with_message!("Heap allocation size not found for layout = {:#x?}", layout);
        };

        let res = get_global_heap_allocator().alloc(layout);
        let Ok(slot) = res else {
            return core::ptr::null_mut();
        };

        if required_slot.size() != slot.size()
            || slot.size() < layout.size()
            || !(slot.as_ptr() as Vaddr).is_multiple_of(layout.align())
        {
            abort_with_message!(
                "Heap allocation mismatch: slot ptr = {:p}, size = {:x}; layout = {:#x?}; required_slot = {:#x?}",
                slot.as_ptr(),
                slot.size(),
                layout,
                required_slot,
            );
        }

        slot.as_ptr()
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let Some(required_slot) = slot_size_from_layout(layout) else {
            abort_with_message!(
                "Heap deallocation size not found for layout = {:#x?}",
                layout
            );
        };

        let slot = unsafe { HeapSlot::new(NonNull::new_unchecked(ptr), required_slot) };
        let res = get_global_heap_allocator().dealloc(slot);

        if res.is_err() {
            abort_with_message!(
                "Heap deallocation error, ptr = {:p}, layout = {:#x?}, required_slot = {:#x?}",
                ptr,
                layout,
                required_slot,
            );
        }
    }
}
