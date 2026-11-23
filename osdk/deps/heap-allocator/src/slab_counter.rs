use core::sync::atomic::{AtomicIsize, Ordering};

use ostd::{
    cpu::{PinCurrentCpu, all_cpus, local::StaticCpuLocal},
    irq,
};

pub(crate) struct SlabCounter {
    per_cpu_counter: &'static StaticCpuLocal<AtomicIsize>,
}

impl SlabCounter {
    pub(crate) const fn new(per_cpu_counter: &'static StaticCpuLocal<AtomicIsize>) -> Self {
        Self { per_cpu_counter }
    }

    pub(crate) fn add(&self, delta: usize) {
        let guard = irq::disable_local();
        self.per_cpu_counter
            .get_on_cpu(guard.current_cpu())
            .fetch_add(delta as isize, Ordering::Relaxed);
    }

    pub(crate) fn sub(&self, delta: usize) {
        let guard = irq::disable_local();
        self.per_cpu_counter
            .get_on_cpu(guard.current_cpu())
            .fetch_sub(delta as isize, Ordering::Relaxed);
    }

    pub(crate) fn get(&self) -> usize {
        let mut total: isize = 0;
        for cpu in all_cpus() {
            total =
                total.wrapping_add(self.per_cpu_counter.get_on_cpu(cpu).load(Ordering::Relaxed));
        }
        if total < 0 { 0 } else { total as usize }
    }
}
