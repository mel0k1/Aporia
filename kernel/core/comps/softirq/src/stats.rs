use aster_util::per_cpu_counter::PerCpuCounter;
use ostd::cpu::CpuId;
use spin::Once;

use super::SoftIrqLine;

pub(super) const NR_IRQ_LINES: usize = 256;

pub(super) static IRQ_COUNTERS: Once<[PerCpuCounter; NR_IRQ_LINES]> = Once::new();

pub fn iter_irq_counts_across_all_cpus() -> impl Iterator<Item = usize> {
    let irq_counters = IRQ_COUNTERS.get().unwrap();
    irq_counters.iter().map(|counter| counter.sum_all_cpus())
}

pub(super) fn process_statistic(irq_num: u8) {
    IRQ_COUNTERS.get().unwrap()[irq_num as usize].add_on_cpu(CpuId::current_racy(), 1);
}

pub fn iter_softirq_counts_across_all_cpus() -> impl Iterator<Item = usize> {
    (0..SoftIrqLine::NR_LINES).map(|i| {
        let soft_irq_line = SoftIrqLine::get(i);
        soft_irq_line
            .counter
            .get()
            .map(PerCpuCounter::sum_all_cpus)
            .unwrap_or(0)
    })
}

pub fn iter_softirq_counts_on_cpu(cpuid: CpuId) -> impl Iterator<Item = usize> {
    (0..SoftIrqLine::NR_LINES).map(move |i| {
        let soft_irq_line = SoftIrqLine::get(i);
        soft_irq_line
            .counter
            .get()
            .map(|c| c.get_on_cpu(cpuid))
            .unwrap_or(0)
    })
}
