#![no_std]
#![deny(unsafe_code)]

#[cfg(ktest)]
extern crate alloc;

use core::alloc::Layout;

use ostd::{
    cpu::PinCurrentCpu,
    irq,
    mm::{Paddr, frame::GlobalFrameAllocator},
};

mod cache;
mod chunk;
mod pools;
mod set;
mod smp_counter;

#[cfg(ktest)]
mod test;

fast_smp_counter! {
    pub static TOTAL_FREE_SIZE: usize;
}

pub fn load_total_free_size() -> usize {
    TOTAL_FREE_SIZE.get()
}

pub struct FrameAllocator;

impl GlobalFrameAllocator for FrameAllocator {
    fn alloc(&self, layout: Layout) -> Option<Paddr> {
        let guard = irq::disable_local();
        let res = cache::alloc(&guard, layout);
        if res.is_some() {
            TOTAL_FREE_SIZE.sub(guard.current_cpu(), layout.size());
        }
        res
    }

    fn dealloc(&self, addr: Paddr, size: usize) {
        let guard = irq::disable_local();
        TOTAL_FREE_SIZE.add(guard.current_cpu(), size);
        cache::dealloc(&guard, addr, size);
    }

    fn add_free_memory(&self, addr: Paddr, size: usize) {
        let guard = irq::disable_local();
        TOTAL_FREE_SIZE.add(guard.current_cpu(), size);
        pools::add_free_memory(&guard, addr, size);
    }
}
