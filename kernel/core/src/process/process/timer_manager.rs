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

/// Updates the CPU time recorded in the CPU clocks of current Process.
///
/// This function will be invoked at the system timer interrupt, and
/// invoke the callbacks of expired timers which are based on the updated
/// CPU clock.
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

/// Registers a function to update the CPU clock in processes and
/// threads during the system timer interrupt.
pub(super) fn init_on_each_cpu() {
    timer::register_callback_on_cpu(update_cpu_time);
}

/// Represents timer resources and utilities for a POSIX process.
pub(crate) struct PosixTimerManager {
    /// A real-time countdown timer, measuring in wall clock time.
    alarm_timer: Arc<Timer>,
    /// A timer based on user CPU clock.
    virtual_timer: Arc<Timer>,
    /// A timer based on the profiling clock.
    prof_timer: Arc<Timer>,
    /// An ID allocator to allocate unique timer IDs.
    id_allocator: Mutex<IdAlloc>,
    /// A container managing all POSIX timers created by `timer_create()` syscall
    /// within the process context.
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

    /// Gets the alarm timer of the corresponding process.
    pub(crate) fn alarm_timer(&self) -> &Arc<Timer> {
        &self.alarm_timer
    }

    /// Gets the virtual timer of the corresponding process.
    pub(crate) fn virtual_timer(&self) -> &Arc<Timer> {
        &self.virtual_timer
    }

    /// Gets the profiling timer of the corresponding process.
    pub(crate) fn prof_timer(&self) -> &Arc<Timer> {
        &self.prof_timer
    }

    /// Creates a timer based on the profiling CPU clock of the current process.
    pub(crate) fn create_prof_timer<F: TimerFn>(&self, func: F) -> Arc<Timer<F>> {
        self.prof_timer.timer_manager().create_timer(func)
    }

    /// Creates a timer based on the user CPU clock of the current process.
    pub(crate) fn create_virtual_timer<F: TimerFn>(&self, func: F) -> Arc<Timer<F>> {
        self.virtual_timer.timer_manager().create_timer(func)
    }

    /// Adds a POSIX timer to the managed `posix_timers`, and allocate a timer ID for this timer.
    /// Returns the timer ID, or `None` if allocation failed.
    pub(crate) fn add_posix_timer(&self, posix_timer: Arc<Timer>) -> Option<timer_t> {
        let mut timers = self.posix_timers.lock();

        let timer_id = self.id_allocator.lock().alloc()?;
        if timers.len() <= timer_id {
            timers.resize(timer_id + 1, None);
        }

        timers[timer_id] = Some(posix_timer);
        Some(timer_id as timer_t)
    }

    /// Finds a POSIX timer by the input `timer_id`.
    pub(crate) fn find_posix_timer(&self, timer_id: timer_t) -> Option<Arc<Timer>> {
        let timer_id = timer_id as usize;
        let timers = self.posix_timers.lock();
        if timer_id >= timers.len() {
            return None;
        }

        timers[timer_id].clone()
    }

    /// Removes the POSIX timer with the ID `timer_id`.
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
