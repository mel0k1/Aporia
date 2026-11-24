//! This module offers `/proc/stat` file support, which provides
//! information about kernel system statistics.
//!
//! Reference: <https://man7.org/linux/man-pages/man5/proc_stat.5.html>

use aster_softirq::{
    iter_irq_counts_across_all_cpus, iter_softirq_counts_across_all_cpus, softirq_id::*,
};
use aster_time::read_start_time;
use aster_util::printer::VmPrinter;
use ostd::util::id_set::Id;

use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps},
        vfs::inode::Inode,
    },
    prelude::*,
    process::collect_process_creation_count,
    sched::nr_queued_and_running,
    thread::collect_context_switch_count,
    time::cpu_time_stats::CpuTimeStatsManager,
};

/// Represents the inode at `/proc/stat`.
pub(super) struct StatFileOps;

impl StatFileOps {
    pub(super) fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self, parent, mkmod!(a+r))
    }

    fn print_stats(printer: &mut VmPrinter) -> Result<()> {
        let stats_manager = CpuTimeStatsManager::singleton();

        let global_stats = stats_manager.collect_stats_on_all_cpus();
        writeln!(
            printer,
            "cpu {} {} {} {} {} {} {} {} {} {}",
            global_stats.user.as_u64(),
            global_stats.nice.as_u64(),
            global_stats.system.as_u64(),
            global_stats.idle.as_u64(),
            global_stats.iowait.as_u64(),
            global_stats.irq.as_u64(),
            global_stats.softirq.as_u64(),
            global_stats.steal.as_u64(),
            global_stats.guest.as_u64(),
            global_stats.guest_nice.as_u64()
        )?;

        for cpu_id in ostd::cpu::all_cpus() {
            let cpu_stats = stats_manager.collect_stats_on_cpu(cpu_id);
            writeln!(
                printer,
                "cpu{} {} {} {} {} {} {} {} {} {} {}",
                cpu_id.as_usize(),
                cpu_stats.user.as_u64(),
                cpu_stats.nice.as_u64(),
                cpu_stats.system.as_u64(),
                cpu_stats.idle.as_u64(),
                cpu_stats.iowait.as_u64(),
                cpu_stats.irq.as_u64(),
                cpu_stats.softirq.as_u64(),
                cpu_stats.steal.as_u64(),
                cpu_stats.guest.as_u64(),
                cpu_stats.guest_nice.as_u64()
            )?;
        }

        let irq_stats = iter_irq_counts_across_all_cpus();
        let mut total_irqs = 0usize;
        let mut irq_counts = Vec::new();
        for count in irq_stats {
            total_irqs += count;
            irq_counts.push(count);
        }
        write!(printer, "intr {}", total_irqs)?;
        for count in irq_counts {
            write!(printer, " {}", count)?;
        }

        writeln!(printer)?;

        let context_switches: usize = collect_context_switch_count();
        writeln!(printer, "ctxt {}", context_switches)?;

        let boot_time = read_start_time().since_epoch().as_secs();
        writeln!(printer, "btime {}", boot_time)?;

        writeln!(printer, "processes {}", collect_process_creation_count())?;

        let (_, running_count) = nr_queued_and_running();
        writeln!(printer, "procs_running {}", running_count)?;

        writeln!(printer, "procs_blocked {}", 0)?;

        let softirq_stats = iter_softirq_counts_across_all_cpus();
        let softirq_stats: Vec<usize> = softirq_stats.collect();
        let total_softirqs: usize = softirq_stats.iter().sum();

        writeln!(
            printer,
            "softirq {} {} {} {} {} {} {} {} {} {} {}",
            total_softirqs,
            softirq_stats[TASKLESS_URGENT_SOFTIRQ_ID as usize],
            softirq_stats[TIMER_SOFTIRQ_ID as usize],
            softirq_stats[TASKLESS_SOFTIRQ_ID as usize],
            softirq_stats[NETWORK_TX_SOFTIRQ_ID as usize],
            softirq_stats[NETWORK_RX_SOFTIRQ_ID as usize],
            0usize,
            0usize,
            0usize,
            0usize,
            0usize,
        )?;

        Ok(())
    }
}

impl ProcFileOps for StatFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        Self::print_stats(&mut printer)?;

        Ok(printer.bytes_written())
    }
}
