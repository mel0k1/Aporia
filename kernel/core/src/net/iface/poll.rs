use alloc::sync::Arc;
use core::time::Duration;

use ostd::{debug, timer::Jiffies};

use super::{Iface, iter_all_ifaces};
use crate::{
    sched::{Nice, SchedPolicy},
    thread::kernel_thread::ThreadOptions,
    time::wait::WaitTimeout,
};

pub(in crate::net) fn init_in_first_kthread() {
    for iface in iter_all_ifaces() {
        spawn_background_poll_thread(iface.clone());
    }
}

pub(super) fn poll_ifaces() {
    for iface in iter_all_ifaces() {
        iface.poll();
    }
}

fn spawn_background_poll_thread(iface: Arc<Iface>) {
    let task_fn = move || {
        debug!("spawn background poll thread for {:?}", iface.name());

        let sched_poll = iface.sched_poll();
        let wait_queue = sched_poll.polling_wait_queue();

        loop {
            let next_poll_at_ms = if let Some(next_poll_at_ms) = sched_poll.next_poll_at_ms() {
                next_poll_at_ms
            } else {
                wait_queue.wait_until(|| sched_poll.next_poll_at_ms())
            };

            let now_as_ms = Jiffies::elapsed().as_duration().as_millis() as u64;

            if now_as_ms >= next_poll_at_ms {
                iface.poll();
                continue;
            }

            let duration = Duration::from_millis(next_poll_at_ms - now_as_ms);
            let _ = wait_queue.wait_until_or_timeout(
                || (sched_poll.next_poll_at_ms()? < next_poll_at_ms).then_some(()),
                &duration,
            );
        }
    };

    ThreadOptions::new(task_fn)
        .sched_policy(SchedPolicy::Fair(Nice::MIN))
        .spawn();
}
