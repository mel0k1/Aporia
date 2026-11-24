use alloc::sync::Arc;
use core::{ptr::NonNull, sync::atomic::Ordering};

use super::{POST_SCHEDULE_HANDLER, PRE_SCHEDULE_HANDLER, Task};
use crate::{
    arch::task::{context_switch, first_context_switch},
    cpu::local::CpuLocalPtr,
    cpu_local_cell,
    irq::DisabledLocalIrqGuard,
};

cpu_local_cell! {
    /// The `Arc<Task>` (casted by [`Arc::into_raw`]) that is the current task.
    static CURRENT_TASK_PTR: CpuLocalPtr<Task> = CpuLocalPtr::null();
    /// The previous task on the processor before switching to the current task.
    /// It is used for delayed resource release since it would be the current
    /// task's job to recycle the previous resources.
    static PREVIOUS_TASK_PTR: CpuLocalPtr<Task> = CpuLocalPtr::null();
}

/// Returns a pointer to the current task running on the processor.
///
/// It returns `None` if the function is called in the bootstrap context.
pub(super) fn current_task() -> Option<NonNull<Task>> {
    NonNull::new(CURRENT_TASK_PTR.load().as_ptr().cast_mut())
}

/// Calls this function to switch to other task
///
/// If current task is none, then it will use the default task context and it
/// will not return to this function again.
///
/// # Panics
///
/// This function will panic if called while holding preemption locks or with
/// local IRQ disabled.
#[track_caller]
pub(super) fn switch_to_task(next_task: Arc<Task>) {
    super::atomic_mode::might_sleep();

    unsafe {
        crate::sync::finish_grace_period();
    }

    let irq_guard = crate::irq::disable_local();

    before_switching_to(&next_task, &irq_guard);

    let next_task_ctx_ptr = next_task.ctx().get().cast_const();

    let current_task_ptr = CURRENT_TASK_PTR.load().as_ptr();
    CURRENT_TASK_PTR.store(CpuLocalPtr::new(Arc::into_raw(next_task)));
    debug_assert!(PREVIOUS_TASK_PTR.load().as_ptr().is_null());
    PREVIOUS_TASK_PTR.store(CpuLocalPtr::new(current_task_ptr));

    core::mem::forget(irq_guard);

    let current_task_ctx_ptr = if !current_task_ptr.is_null() {
        let current_task = unsafe { &*current_task_ptr };

        current_task.ctx.get()
    } else {
        unsafe { first_context_switch(next_task_ctx_ptr) };

        unreachable!("`first_context_switch` should never return");
    };

    unsafe {
        context_switch(next_task_ctx_ptr, current_task_ctx_ptr);
    }

    unsafe { after_switching_to() };
}

fn before_switching_to(next_task: &Task, irq_guard: &DisabledLocalIrqGuard) {
    if let Some(handler) = PRE_SCHEDULE_HANDLER.get() {
        handler(irq_guard);
    }

    next_task.kstack.flush_tlb(irq_guard);

    while next_task
        .switched_to_cpu
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
        .is_err()
    {
        crate::warn!("Switching to a task already running in the foreground");
        core::hint::spin_loop();
    }
}

/// Does cleanups after switching to a task.
///
/// # Safety
///
/// This function must be called only once after switching to a task.
pub(super) unsafe fn after_switching_to() {
    let prev = PREVIOUS_TASK_PTR.load().as_ptr();
    let prev = if !prev.is_null() {
        PREVIOUS_TASK_PTR.store(CpuLocalPtr::null());

        let prev_task = unsafe { Arc::from_raw(prev) };

        prev_task.switched_to_cpu.store(false, Ordering::Release);

        Some(prev_task)
    } else {
        None
    };

    if let Some(handler) = POST_SCHEDULE_HANDLER.get() {
        handler();
    }

    crate::arch::irq::enable_local();

    drop(prev);
}
