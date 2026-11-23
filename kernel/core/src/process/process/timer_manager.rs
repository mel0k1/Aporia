use alloc::{
    boxed::Box,
    sync::{Arc, Weak},
    vec::Vec,
};

use id_alloc::IdAlloc;
use ostd::{cpu::PrivilegeLevel, irq::InterruptLevel, sync::Mutex, timer};

use super::Process;
use crate::{
    fs::cgroupfs::{CpuStatKind, charge_cpu_time},
    process::{
        posix_thread::AsPosixThread,
        signal::{
            constants::{SIGALRM, SIGPROF, SIGVTALRM},
            sig_num::SigNum,
            signals::kernel::KernelSignal,
        },
    },
    thread::{
        Thread,
        work_queue::{submit_work_item, work_item::WorkItem},
    },
    time::{
        Timer, TimerManager,
        clocks::{ProfClock, RealTimeClock},
        timer::{TimerFn, TimerGuard},
        timer_t,
    },
};

fn update_cpu_time() {
    let is_kernel_interrupted = {
        let interrupt_level = InterruptLevel::current();
        let InterruptLevel::L1(cpu_priv_at_irq) = interrupt_level else {
            return;
        };
        cpu_priv_at_irq == PrivilegeLevel::Kernel
    };

    let Some(current_thread) = Thread::current() else {
        return;
    };
    let Some(posix_thread) = current_thread.as_posix_thread() else {
        return;
    };
    let Some(process) = posix_thread.weak_process().upgrade() else {
        return;
    };

    let timer_manager = process.timer_manager();
    if is_kernel_interrupted {
        posix_thread.prof_clock().kernel_clock().add_jiffies(1);
        process.prof_clock().kernel_clock().add_jiffies(1);
        charge_cpu_time(&process, CpuStatKind::System);
    } else {
        posix_thread.prof_clock().user_clock().add_jiffies(1);
        process.prof_clock().user_clock().add_jiffies(1);
        charge_cpu_time(&process, CpuStatKind::User);
        timer_manager
            .virtual_timer()
            .timer_manager()
            .process_expired_timers();
    }
    timer_manager
        .prof_timer()
        .timer_manager()
        .process_expired_timers();
    posix_thread.process_expired_timers();
}

pub(super) fn init_on_each_cpu() {
    timer::register_callback_on_cpu(update_cpu_time);
}

pub(crate) struct PosixTimerManager {
    alarm_timer: Arc<Timer>,
    virtual_timer: Arc<Timer>,
    prof_timer: Arc<Timer>,
    id_allocator: Mutex<IdAlloc>,
    posix_timers: Mutex<Vec<Option<Arc<Timer>>>>,
}

fn create_process_timer_callback(
    process_ref: &Weak<Process>,
    signum: SigNum,
) -> impl Fn(TimerGuard) + 'static {
    let current_process = process_ref.clone();
    let sent_signal = move || {
        let signal = KernelSignal::new(signum);
        if let Some(process) = current_process.upgrade() {
            process.enqueue_signal(Box::new(signal));
        }
    };

    let work_func = Box::new(sent_signal);
    let work_item = WorkItem::new(work_func);

    move |_guard: TimerGuard| {
        submit_work_item(
            work_item.clone(),
            crate::thread::work_queue::WorkPriority::High,
        );
    }
}

impl PosixTimerManager {
    pub(super) fn new(prof_clock: &Arc<ProfClock>, process_ref: &Weak<Process>) -> Self {
        const MAX_NUM_OF_POSIX_TIMERS: usize = 10000;
        const { assert!(MAX_NUM_OF_POSIX_TIMERS <= timer_t::MAX as usize) };

        let alarm_timer = RealTimeClock::timer_manager()
            .create_timer(create_process_timer_callback(process_ref, SIGALRM));
        let virtual_timer = TimerManager::new(prof_clock.user_clock().clone())
            .create_timer(create_process_timer_callback(process_ref, SIGVTALRM));
        let prof_timer = TimerManager::new(prof_clock.clone())
            .create_timer(create_process_timer_callback(process_ref, SIGPROF));

        Self {
            alarm_timer,
            virtual_timer,
            prof_timer,
            id_allocator: Mutex::new(IdAlloc::with_capacity(MAX_NUM_OF_POSIX_TIMERS)),
            posix_timers: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn alarm_timer(&self) -> &Arc<Timer> {
        &self.alarm_timer
    }

    pub(crate) fn virtual_timer(&self) -> &Arc<Timer> {
        &self.virtual_timer
    }

    pub(crate) fn prof_timer(&self) -> &Arc<Timer> {
        &self.prof_timer
    }

    pub(crate) fn create_prof_timer<F: TimerFn>(&self, func: F) -> Arc<Timer<F>> {
        self.prof_timer.timer_manager().create_timer(func)
    }

    pub(crate) fn create_virtual_timer<F: TimerFn>(&self, func: F) -> Arc<Timer<F>> {
        self.virtual_timer.timer_manager().create_timer(func)
    }

    pub(crate) fn add_posix_timer(&self, posix_timer: Arc<Timer>) -> Option<timer_t> {
        let mut timers = self.posix_timers.lock();
        let timer_id = self.id_allocator.lock().alloc()?;
        if timers.len() <= timer_id {
            timers.resize(timer_id + 1, None);
        }
        timers[timer_id] = Some(posix_timer);
        Some(timer_id as timer_t)
    }

    pub(crate) fn find_posix_timer(&self, timer_id: timer_t) -> Option<Arc<Timer>> {
        let timer_id = timer_id as usize;
        let timers = self.posix_timers.lock();
        if timer_id >= timers.len() {
            return None;
        }

        timers[timer_id].clone()
    }

    pub(crate) fn remove_posix_timer(&self, timer_id: timer_t) -> Option<Arc<Timer>> {
        let timer_id = timer_id as usize;
        let mut timers = self.posix_timers.lock();
        if timer_id >= timers.len() {
            return None;
        }

        let timer = timers[timer_id].take();
        if timer.is_some() {
            self.id_allocator.lock().free(timer_id);
        }
        timer
    }
}
