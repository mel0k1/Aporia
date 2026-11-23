use aster_util::per_cpu_counter::PerCpuCounter;
use spin::Once;

pub(super) static PROCESS_CREATION_COUNTER: Once<PerCpuCounter> = Once::new();

pub(crate) fn collect_process_creation_count() -> usize {
    PROCESS_CREATION_COUNTER.get().unwrap().sum_all_cpus()
}

pub(super) fn init() {
    PROCESS_CREATION_COUNTER.call_once(PerCpuCounter::new);
}
