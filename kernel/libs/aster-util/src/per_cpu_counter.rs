use core::sync::atomic::{AtomicIsize, Ordering};

use osdk_heap_allocator::{CpuLocalBox, alloc_cpu_local};
use ostd::cpu::{CpuId, all_cpus};

pub struct PerCpuCounter {
    per_cpu_counter: CpuLocalBox<AtomicIsize>,
}

impl PerCpuCounter {
    pub fn new() -> Self {
        Self {
            per_cpu_counter: alloc_cpu_local(|_| AtomicIsize::new(0)).unwrap(),
        }
    }

    pub fn add_on_cpu(&self, on_cpu: CpuId, increment: isize) {
        self.per_cpu_counter
            .get_on_cpu(on_cpu)
            .fetch_add(increment, Ordering::Relaxed);
    }

    pub fn sum_all_cpus(&self) -> usize {
        let mut total: isize = 0;
        for cpu in all_cpus() {
            total =
                total.wrapping_add(self.per_cpu_counter.get_on_cpu(cpu).load(Ordering::Relaxed));
        }
        if total < 0 { 0 } else { total as usize }
    }

    pub fn get_on_cpu(&self, cpu: CpuId) -> usize {
        let val = self.per_cpu_counter.get_on_cpu(cpu).load(Ordering::Relaxed);
        if val < 0 { 0 } else { val as usize }
    }
}

impl Default for PerCpuCounter {
    fn default() -> Self {
        Self::new()
    }
}
