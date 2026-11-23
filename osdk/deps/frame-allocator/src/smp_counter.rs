use core::sync::atomic::{AtomicIsize, Ordering};

use ostd::cpu::{CpuId, all_cpus, local::StaticCpuLocal};

#[macro_export]
macro_rules! fast_smp_counter {
    ($(#[$attr:meta])* $vis:vis static $name:ident : usize;) => { paste::paste!{
        ostd::cpu_local! {
            static [< __LOCAL_COUNTER_ $name >]: core::sync::atomic::AtomicIsize
                = core::sync::atomic::AtomicIsize::new(0);
        }

        $(#[$attr])*
        $vis static $name: $crate::smp_counter::FastSmpCounter =
            $crate::smp_counter::FastSmpCounter::new(
                & [< __LOCAL_COUNTER_ $name >],
            );
    }};
}

pub struct FastSmpCounter {
    per_cpu_counter: &'static StaticCpuLocal<AtomicIsize>,
}

impl FastSmpCounter {
    #[doc(hidden)]
    pub const fn new(per_cpu_counter: &'static StaticCpuLocal<AtomicIsize>) -> Self {
        Self { per_cpu_counter }
    }

    pub fn add(&self, on_cpu: CpuId, a: usize) {
        self.per_cpu_counter
            .get_on_cpu(on_cpu)
            .fetch_add(a as isize, Ordering::Relaxed);
    }

    pub fn sub(&self, on_cpu: CpuId, a: usize) {
        self.per_cpu_counter
            .get_on_cpu(on_cpu)
            .fetch_sub(a as isize, Ordering::Relaxed);
    }

    pub fn get(&self) -> usize {
        let mut total: isize = 0;
        for cpu in all_cpus() {
            total =
                total.wrapping_add(self.per_cpu_counter.get_on_cpu(cpu).load(Ordering::Relaxed));
        }
        if total < 0 { 0 } else { total as usize }
    }
}

#[cfg(ktest)]
mod test {
    use ostd::{cpu::PinCurrentCpu, irq, prelude::*};

    #[ktest]
    fn per_cpu_counter() {
        fast_smp_counter! {
            pub(crate) static FREE_SIZE_COUNTER: usize;
        }

        let guard = irq::disable_local();
        let cur_cpu = guard.current_cpu();
        FREE_SIZE_COUNTER.add(cur_cpu, 10);
        assert_eq!(FREE_SIZE_COUNTER.get(), 10);
        FREE_SIZE_COUNTER.add(cur_cpu, 20);
        assert_eq!(FREE_SIZE_COUNTER.get(), 30);
        FREE_SIZE_COUNTER.sub(cur_cpu, 5);
        assert_eq!(FREE_SIZE_COUNTER.get(), 25);
    }
}
