#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use core::sync::atomic::AtomicIsize;

use ostd::cpu_local;

macro_rules! __log_prefix {
    () => {
        "heap: "
    };
}

mod allocator;
mod cpu_local_allocator;
mod slab_cache;
mod slab_counter;
#[cfg(ktest)]
mod test;

pub use allocator::{HeapAllocator, type_from_layout};
pub use cpu_local_allocator::{CpuLocalBox, alloc_cpu_local};

cpu_local! {
    static LOCAL_TOTAL_SLAB_ALLOCATED: AtomicIsize = AtomicIsize::new(0);
}

pub(crate) static TOTAL_SLAB_ALLOCATED: slab_counter::SlabCounter =
    slab_counter::SlabCounter::new(&LOCAL_TOTAL_SLAB_ALLOCATED);

pub fn load_total_slab_size() -> usize {
    TOTAL_SLAB_ALLOCATED.get()
}
