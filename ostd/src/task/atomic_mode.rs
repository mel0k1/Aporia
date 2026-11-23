use core::sync::atomic::Ordering;

#[track_caller]
pub fn might_sleep() {
    let preempt_count = super::preempt::cpu_local::get_guard_count();
    let is_local_irq_enabled = crate::arch::irq::is_local_enabled();
    if (preempt_count != 0 || !is_local_irq_enabled)
        && !crate::IN_BOOTSTRAP_CONTEXT.load(Ordering::Relaxed)
    {
        panic!(
            "This function might break atomic mode (preempt_count = {}, is_local_irq_enabled = {})",
            preempt_count, is_local_irq_enabled
        );
    }
}

pub unsafe trait InAtomicMode: core::fmt::Debug {}

pub trait AsAtomicModeGuard {
    fn as_atomic_mode_guard(&self) -> &dyn InAtomicMode;
}

impl<G: InAtomicMode> AsAtomicModeGuard for G {
    fn as_atomic_mode_guard(&self) -> &dyn InAtomicMode {
        self
    }
}

impl AsAtomicModeGuard for dyn InAtomicMode + '_ {
    fn as_atomic_mode_guard(&self) -> &dyn InAtomicMode {
        self
    }
}
