use core::sync::atomic::{AtomicBool, Ordering};

use aster_util::per_cpu_counter::PerCpuCounter;
use ostd::{
    cpu::{AtomicCpuSet, CpuId, CpuSet},
    irq::DisabledLocalIrqGuard,
    task::Task,
};

use self::{kernel_thread::AsKernelThread, stats::CONTEXT_SWITCH_COUNTER};
use crate::{
    prelude::*,
    sched::{SchedAttr, SchedPolicy},
};

pub(crate) mod exception;
pub(crate) mod kernel_thread;
pub(crate) mod oops;
mod softirqd;
mod stats;
pub(crate) mod task;
pub(crate) mod work_queue;

pub(crate) use self::stats::collect_context_switch_count;

pub(crate) type Tid = u32;

fn pre_schedule_handler(irq_guard: &DisabledLocalIrqGuard) {
    let Some(task) = Task::current() else {
        return;
    };
    let Some(thread_local) = task.as_thread_local() else {
        return;
    };

    thread_local.supp_user_context().before_schedule(irq_guard);
}

fn post_schedule_handler() {
    CONTEXT_SWITCH_COUNTER
        .get()
        .unwrap()
        .add_on_cpu(CpuId::current_racy(), 1);

    let task = Task::current().unwrap();
    if let Some(thread_local) = task.as_thread_local() {
        let vmar = thread_local.vmar().borrow();
        if let Some(vmar) = vmar.as_ref() {
            vmar.vm_space().activate()
        }
    } else if let Some(vmar) = task
        .as_kernel_thread()
        .and_then(|kernel_thread| kernel_thread.vmar())
    {
        vmar.vm_space().activate();
    }
}

pub(super) fn init() {
    CONTEXT_SWITCH_COUNTER.call_once(PerCpuCounter::new);
    ostd::task::inject_pre_schedule_handler(pre_schedule_handler);
    ostd::task::inject_post_schedule_handler(post_schedule_handler);
    ostd::mm::fault::inject_user_page_fault_handler(exception::page_fault_handler);
}

pub(super) fn init_in_first_kthread() {
    work_queue::init_in_first_kthread();
    softirqd::init_in_first_kthread();
}

#[derive(Debug)]
pub(crate) struct Thread {
    task: Weak<Task>,
    data: Box<dyn Send + Sync + Any>,

    is_exited: AtomicBool,
    cpu_affinity: AtomicCpuSet,
    sched_attr: SchedAttr,
}

impl Thread {
    pub(crate) fn new(
        task: Weak<Task>,
        data: impl Send + Sync + Any,
        cpu_affinity: CpuSet,
        sched_policy: SchedPolicy,
    ) -> Self {
        Thread {
            task,
            data: Box::new(data),
            is_exited: AtomicBool::new(false),
            cpu_affinity: AtomicCpuSet::new(cpu_affinity),
            sched_attr: SchedAttr::new(sched_policy),
        }
    }

    pub(crate) fn current() -> Option<Arc<Self>> {
        Task::current()?.as_thread().cloned()
    }

    #[expect(dead_code)]
    pub(crate) fn task(&self) -> Arc<Task> {
        self.task.upgrade().unwrap()
    }

    #[track_caller]
    pub(crate) fn run(&self) {
        self.task.upgrade().unwrap().run();
    }

    pub(crate) fn is_exited(&self) -> bool {
        self.is_exited.load(Ordering::Acquire)
    }

    pub(super) fn exit(&self) {
        self.is_exited.store(true, Ordering::Release);
    }

    pub(crate) fn atomic_cpu_affinity(&self) -> &AtomicCpuSet {
        &self.cpu_affinity
    }

    pub(crate) fn sched_attr(&self) -> &SchedAttr {
        &self.sched_attr
    }

    #[track_caller]
    pub(crate) fn yield_now() {
        Task::yield_now()
    }

    #[cfg_attr(not(ktest), expect(dead_code))]
    #[track_caller]
    pub(crate) fn join(&self) {
        while !self.is_exited() {
            Self::yield_now();
        }
    }

    pub(crate) fn data(&self) -> &(dyn Send + Sync + Any) {
        &*self.data
    }
}

pub(crate) trait AsThread {
    fn as_thread(&self) -> Option<&Arc<Thread>>;
}

impl AsThread for Task {
    fn as_thread(&self) -> Option<&Arc<Thread>> {
        self.data().downcast_ref::<Arc<Thread>>()
    }
}
