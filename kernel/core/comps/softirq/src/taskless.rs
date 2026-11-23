use alloc::{boxed::Box, sync::Arc};
use core::{
    cell::RefCell,
    ops::DerefMut,
    sync::atomic::{AtomicBool, Ordering},
};

use intrusive_collections::{LinkedList, LinkedListAtomicLink, intrusive_adapter};
use ostd::{cpu::local::StaticCpuLocal, cpu_local, irq};

use super::{
    SoftIrqLine,
    softirq_id::{TASKLESS_SOFTIRQ_ID, TASKLESS_URGENT_SOFTIRQ_ID},
};

pub struct Taskless {
    is_scheduled: AtomicBool,
    is_running: AtomicBool,
    callback: Box<dyn Fn() + Send + Sync + 'static>,
    is_disabled: AtomicBool,
    link: LinkedListAtomicLink,
}

intrusive_adapter!(TasklessAdapter = Arc<Taskless>: Taskless { link: LinkedListAtomicLink });

cpu_local! {
    static TASKLESS_LIST: RefCell<LinkedList<TasklessAdapter>> =
        RefCell::new(LinkedList::new(TasklessAdapter::NEW));
    static TASKLESS_URGENT_LIST: RefCell<LinkedList<TasklessAdapter>> =
        RefCell::new(LinkedList::new(TasklessAdapter::NEW));
}

impl Taskless {
    pub fn new<F>(callback: F) -> Arc<Self>
    where
        F: Fn() + Send + Sync + 'static,
    {
        Arc::new(Self {
            is_scheduled: AtomicBool::new(false),
            is_running: AtomicBool::new(false),
            callback: Box::new(callback),
            is_disabled: AtomicBool::new(false),
            link: LinkedListAtomicLink::new(),
        })
    }

    pub fn schedule(self: &Arc<Self>) {
        do_schedule(self, &TASKLESS_LIST);
        SoftIrqLine::get(TASKLESS_SOFTIRQ_ID).raise();
    }

    pub fn schedule_urgent(self: &Arc<Self>) {
        do_schedule(self, &TASKLESS_URGENT_LIST);
        SoftIrqLine::get(TASKLESS_URGENT_SOFTIRQ_ID).raise();
    }

    pub fn enable(&self) {
        self.is_disabled.store(false, Ordering::Release);
    }

    pub fn disable(&self) {
        self.is_disabled.store(true, Ordering::Release);
    }
}

fn do_schedule(
    taskless: &Arc<Taskless>,
    taskless_list: &'static StaticCpuLocal<RefCell<LinkedList<TasklessAdapter>>>,
) {
    if taskless.is_disabled.load(Ordering::Acquire) {
        return;
    }
    if taskless
        .is_scheduled
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        return;
    }
    let irq_guard = irq::disable_local();
    taskless_list
        .get_with(&irq_guard)
        .borrow_mut()
        .push_front(taskless.clone());
}

pub(super) fn init() {
    SoftIrqLine::get(TASKLESS_URGENT_SOFTIRQ_ID)
        .enable(|| taskless_softirq_handler(&TASKLESS_URGENT_LIST, TASKLESS_URGENT_SOFTIRQ_ID));
    SoftIrqLine::get(TASKLESS_SOFTIRQ_ID)
        .enable(|| taskless_softirq_handler(&TASKLESS_LIST, TASKLESS_SOFTIRQ_ID));
}

fn taskless_softirq_handler(
    taskless_list: &'static StaticCpuLocal<RefCell<LinkedList<TasklessAdapter>>>,
    softirq_id: u8,
) {
    let mut processing_list = {
        let irq_guard = irq::disable_local();
        let guard = taskless_list.get_with(&irq_guard);
        let mut list_mut = guard.borrow_mut();
        LinkedList::take(list_mut.deref_mut())
    };

    while let Some(taskless) = processing_list.pop_back() {
        if taskless
            .is_running
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            let irq_guard = irq::disable_local();
            taskless_list
                .get_with(&irq_guard)
                .borrow_mut()
                .push_front(taskless);
            SoftIrqLine::get(softirq_id).raise();
            continue;
        }

        taskless.is_scheduled.store(false, Ordering::Release);

        (taskless.callback)();
        taskless.is_running.store(false, Ordering::Release);
    }
}

#[cfg(ktest)]
mod test {
    use core::sync::atomic::AtomicUsize;

    use ostd::prelude::*;
    use spin::Once;

    use super::*;

    fn init() {
        static INIT: Once<()> = Once::new();

        INIT.call_once(|| super::super::init().unwrap());
    }

    #[ktest]
    fn schedule_taskless() {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        const SCHEDULE_TIMES: usize = 10;

        fn add_counter() {
            COUNTER.fetch_add(1, Ordering::Relaxed);
        }

        init();
        let taskless = Taskless::new(add_counter);
        let mut counter = 0;

        while !taskless.is_scheduled.load(Ordering::Acquire) {
            taskless.schedule();
            counter += 1;
            if counter == SCHEDULE_TIMES {
                break;
            }
        }

        while taskless.is_running.load(Ordering::Acquire)
            || taskless.is_scheduled.load(Ordering::Acquire)
        {
            core::hint::spin_loop()
        }

        assert_eq!(counter, COUNTER.load(Ordering::Relaxed));
    }
}
