mod fifo_scheduler;
pub mod info;

use spin::Once;

use super::{Task, preempt::cpu_local, processor};
use crate::{
    cpu::{CpuId, CpuSet, PinCurrentCpu},
    prelude::*,
    task::disable_preempt,
    timer,
};

pub fn inject_scheduler(scheduler: &'static dyn Scheduler<Task>) {
    let mut is_new = false;
    SCHEDULER.call_once(|| {
        is_new = true;
        scheduler
    });
    assert!(is_new, "a scheduler has already been initialized");
}

pub fn enable_preemption_on_cpu() {
    timer::register_callback_on_cpu(|| {
        scheduler_singleton().mut_local_rq_with(&mut |local_rq| {
            let should_pick_next = local_rq.update_current(UpdateFlags::Tick);
            if should_pick_next {
                cpu_local::set_need_preempt();
            }
        })
    });
}

static SCHEDULER: Once<&'static dyn Scheduler<Task>> = Once::new();

fn scheduler_singleton() -> &'static dyn Scheduler<Task> {
    *SCHEDULER.call_once(|| fifo_scheduler::new_instance())
}

pub trait Scheduler<T = Task>: Sync + Send {
    fn enqueue(&self, runnable: Arc<T>, flags: EnqueueFlags) -> Option<CpuId>;

    fn local_rq_with(&self, f: &mut dyn FnMut(&dyn LocalRunQueue<T>));

    fn mut_local_rq_with(&self, f: &mut dyn FnMut(&mut dyn LocalRunQueue<T>));
}

pub trait LocalRunQueue<T = Task> {
    fn current(&self) -> Option<&Arc<T>>;

    #[must_use]
    fn update_current(&mut self, flags: UpdateFlags) -> bool;

    fn pick_next(&mut self) -> &Arc<T> {
        self.try_pick_next().unwrap()
    }

    fn try_pick_next(&mut self) -> Option<&Arc<T>>;

    fn dequeue_current(&mut self) -> Option<Arc<T>>;
}

#[derive(Clone, Copy, PartialEq)]
pub enum EnqueueFlags {
    Spawn,
    Wake,
}

#[derive(Clone, Copy, PartialEq)]
pub enum UpdateFlags {
    Tick,
    Wait,
    Yield,
    Exit,
}

#[track_caller]
pub(crate) fn might_preempt() {
    if !cpu_local::should_preempt() {
        return;
    }
    reschedule(|local_rq| {
        let next_task_opt = local_rq.try_pick_next();
        if let Some(next_task) = next_task_opt {
            ReschedAction::SwitchTo(next_task.clone())
        } else {
            ReschedAction::DoNothing
        }
    })
}

#[track_caller]
pub(crate) fn park_current<F>(has_unparked: F)
where
    F: Fn() -> bool,
{
    let mut current = None;
    let mut is_first_try = true;

    reschedule(|local_rq: &mut dyn LocalRunQueue| {
        let next_task_opt = if is_first_try {
            if has_unparked() {
                return ReschedAction::DoNothing;
            }
            is_first_try = false;

            let should_pick_next = local_rq.update_current(UpdateFlags::Wait);
            current = local_rq.dequeue_current();
            should_pick_next.then(|| local_rq.pick_next())
        } else {
            local_rq.try_pick_next()
        };

        if let Some(next_task) = next_task_opt {
            if Arc::ptr_eq(current.as_ref().unwrap(), next_task) {
                return ReschedAction::DoNothing;
            }
            return ReschedAction::SwitchTo(next_task.clone());
        }

        ReschedAction::Retry
    });
}

pub(crate) fn unpark_target(runnable: Arc<Task>) {
    let preempt_cpu = scheduler_singleton().enqueue(runnable, EnqueueFlags::Wake);
    if let Some(preempt_cpu_id) = preempt_cpu {
        set_need_preempt(preempt_cpu_id);
    }
}

#[track_caller]
pub(super) fn run_new_task(runnable: Arc<Task>) {
    let preempt_cpu = scheduler_singleton().enqueue(runnable, EnqueueFlags::Spawn);
    if let Some(preempt_cpu_id) = preempt_cpu {
        set_need_preempt(preempt_cpu_id);
    }

    might_preempt();
}

fn set_need_preempt(cpu_id: CpuId) {
    let preempt_guard = disable_preempt();

    if preempt_guard.current_cpu() == cpu_id {
        cpu_local::set_need_preempt();
    } else {
        crate::smp::inter_processor_call(&CpuSet::from(cpu_id), || {
            cpu_local::set_need_preempt();
        });
    }
}

#[track_caller]
pub(super) fn exit_current() -> ! {
    let mut is_first_try = true;

    reschedule(|local_rq: &mut dyn LocalRunQueue| {
        let next_task_opt = if is_first_try {
            is_first_try = false;
            let should_pick_next = local_rq.update_current(UpdateFlags::Exit);
            let _current = local_rq.dequeue_current();
            should_pick_next.then(|| local_rq.pick_next())
        } else {
            local_rq.try_pick_next()
        };

        if let Some(next_task) = next_task_opt {
            ReschedAction::SwitchTo(next_task.clone())
        } else {
            ReschedAction::Retry
        }
    });

    unreachable!()
}

#[track_caller]
pub(super) fn yield_now() {
    reschedule(|local_rq| {
        let should_pick_next = local_rq.update_current(UpdateFlags::Yield);
        let next_task_opt = should_pick_next.then(|| local_rq.pick_next());
        if let Some(next_task) = next_task_opt {
            ReschedAction::SwitchTo(next_task.clone())
        } else {
            ReschedAction::DoNothing
        }
    })
}

#[track_caller]
fn reschedule<F>(mut f: F)
where
    F: FnMut(&mut dyn LocalRunQueue) -> ReschedAction,
{
    cpu_local::clear_need_preempt();

    let next_task = loop {
        let mut action = ReschedAction::DoNothing;
        scheduler_singleton().mut_local_rq_with(&mut |rq| {
            action = f(rq);
        });

        match action {
            ReschedAction::DoNothing => {
                return;
            }
            ReschedAction::Retry => {
                continue;
            }
            ReschedAction::SwitchTo(next_task) => {
                break next_task;
            }
        };
    };

    processor::switch_to_task(next_task);
}

enum ReschedAction {
    DoNothing,
    Retry,
    SwitchTo(Arc<Task>),
}
