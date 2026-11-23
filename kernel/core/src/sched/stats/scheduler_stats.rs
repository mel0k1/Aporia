use ostd::timer;
use spin::Once;

use super::loadavg;

static SCHEDULER_STATS: Once<&'static dyn SchedulerStats> = Once::new();

pub(crate) fn set_stats_from_scheduler(scheduler: &'static dyn SchedulerStats) {
    SCHEDULER_STATS.call_once(|| scheduler);

    timer::register_callback_on_cpu(|| {
        loadavg::update_loadavg(|| nr_queued_and_running().0);
    });
}

pub(crate) trait SchedulerStats: Sync + Send {
    fn nr_queued_and_running(&self) -> (u32, u32);
}

pub(crate) fn nr_queued_and_running() -> (u32, u32) {
    SCHEDULER_STATS.get().unwrap().nr_queued_and_running()
}
