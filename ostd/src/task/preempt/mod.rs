pub(super) mod cpu_local;
mod guard;

pub use self::guard::{DisabledPreemptGuard, disable_preempt};

#[track_caller]
pub fn halt_cpu() {
    crate::task::atomic_mode::might_sleep();

    let irq_guard = crate::irq::disable_local();

    if cpu_local::need_preempt() {
        drop(irq_guard);
    } else {
        core::mem::forget(irq_guard);
        crate::arch::irq::enable_local_and_halt();
    }

    super::scheduler::might_preempt();
}
