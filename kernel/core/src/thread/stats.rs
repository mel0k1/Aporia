use aster_util::per_cpu_counter::PerCpuCounter;
use spin::Once;

pub(super) static CONTEXT_SWITCH_COUNTER: Once<PerCpuCounter> = Once::new();

pub(crate) fn collect_context_switch_count() -> usize {
    CONTEXT_SWITCH_COUNTER.get().unwrap().sum_all_cpus()
}
