#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::{boxed::Box, sync::Arc};
use core::sync::atomic::{AtomicU8, Ordering};

use aster_util::per_cpu_counter::PerCpuCounter;
use component::{ComponentInitError, init_component};
use ostd::{
    cpu::{CpuId, PinCurrentCpu},
    cpu_local, cpu_local_cell,
    irq::{
        DisabledLocalIrqGuard, disable_local, register_bottom_half_handler_l1,
        register_bottom_half_handler_l2,
    },
    sync::{Waiter, Waker},
    task::{Task, disable_preempt},
};
use spin::Once;

use self::{
    lock::{disable_local_bottom_half_with, is_softirq_enabled},
    stats::{IRQ_COUNTERS, NR_IRQ_LINES, process_statistic},
};

mod lock;
pub mod softirq_id;
mod stats;
mod taskless;

pub use self::{
    lock::{BottomHalfDisabled, DisableLocalBottomHalfGuard},
    stats::{
        iter_irq_counts_across_all_cpus, iter_softirq_counts_across_all_cpus,
        iter_softirq_counts_on_cpu,
    },
    taskless::Taskless,
};

pub struct SoftIrqLine {
    id: u8,
    callback: Once<Box<dyn Fn() + 'static + Sync + Send>>,
    counter: Once<PerCpuCounter>,
}

impl SoftIrqLine {
    const NR_LINES: u8 = 8;

    pub fn get(id: u8) -> &'static SoftIrqLine {
        &LINES.get().unwrap()[id as usize]
    }

    const fn new(id: u8) -> Self {
        Self {
            id,
            callback: Once::new(),
            counter: Once::new(),
        }
    }

    pub fn id(&self) -> u8 {
        self.id
    }

    pub fn raise(&self) {
        PENDING_MASK.bitor_assign(1 << self.id);
    }

    pub fn enable<F>(&self, callback: F)
    where
        F: Fn() + 'static + Sync + Send,
    {
        assert!(!self.is_enabled());

        self.counter.call_once(PerCpuCounter::new);
        self.callback.call_once(|| Box::new(callback));
        ENABLED_MASK.fetch_or(1 << self.id, Ordering::Release);
    }

    pub fn is_enabled(&self) -> bool {
        ENABLED_MASK.load(Ordering::Acquire) & (1 << self.id) != 0
    }
}

static LINES: Once<[SoftIrqLine; SoftIrqLine::NR_LINES as usize]> = Once::new();

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    let lines: [SoftIrqLine; SoftIrqLine::NR_LINES as usize] =
        core::array::from_fn(|i| SoftIrqLine::new(i as u8));
    LINES.call_once(|| lines);

    let interrupt_counter: [PerCpuCounter; NR_IRQ_LINES] =
        core::array::from_fn(|_| PerCpuCounter::new());
    IRQ_COUNTERS.call_once(|| interrupt_counter);

    register_bottom_half_handler_l1(process_pending);
    register_bottom_half_handler_l2(process_statistic);
    taskless::init();
    Ok(())
}

#[cfg(ktest)]
pub fn init_for_ktest() {
    init().unwrap();
}

static ENABLED_MASK: AtomicU8 = AtomicU8::new(0);

cpu_local_cell! {
    static PENDING_MASK: u8 = 0;
}

cpu_local! {
    static DAEMON_WAKER: Once<Arc<Waker>> = Once::new();
}

fn process_pending(irq_guard: DisabledLocalIrqGuard, irq_num: u8) -> DisabledLocalIrqGuard {
    process_statistic(irq_num);

    if !is_softirq_enabled() {
        return irq_guard;
    }
    process_all_pending(irq_guard)
}

fn process_all_pending(mut irq_guard: DisabledLocalIrqGuard) -> DisabledLocalIrqGuard {
    const SOFTIRQ_RUN_TIMES: u8 = 5;

    let mut run_time = 0;

    loop {
        let action_mask = {
            let pending_mask = PENDING_MASK.load();
            pending_mask & ENABLED_MASK.load(Ordering::Acquire)
        };

        if action_mask == 0 {
            break;
        }

        if run_time >= SOFTIRQ_RUN_TIMES || Task::need_yield() {
            if let Some(waker) = DAEMON_WAKER.get_with(&irq_guard).get() {
                let _ = waker.wake_up();
            }
            break;
        }
        run_time += 1;

        PENDING_MASK.store(0);

        drop(irq_guard);

        run_callbacks(action_mask);

        irq_guard = disable_local();
    }

    irq_guard
}

pub fn daemon_loop_on_cpu() {
    let (waiter, waker) = Waiter::new_pair();

    let mut preempt_guard = disable_preempt();

    DAEMON_WAKER
        .get_on_cpu(preempt_guard.current_cpu())
        .call_once(|| waker);

    let mut should_wait = false;

    loop {
        if should_wait {
            drop(preempt_guard);
            waiter.wait();
            preempt_guard = disable_preempt();

            should_wait = false;
        }

        let (action_mask, bottom_half_gurad) = {
            let _irq_guard = disable_local();

            let mut pending_mask = PENDING_MASK.load();
            pending_mask &= ENABLED_MASK.load(Ordering::Acquire);

            if pending_mask == 0 {
                should_wait = true;
                continue;
            }

            PENDING_MASK.store(0);

            let bottom_half_guard = disable_local_bottom_half_with(&preempt_guard);

            (pending_mask, bottom_half_guard)
        };

        run_callbacks(action_mask);

        drop(bottom_half_gurad);

        if Task::need_yield() {
            drop(preempt_guard);
            Task::yield_now();
            preempt_guard = disable_preempt();
        }
    }
}

fn run_callbacks(mut action_mask: u8) {
    while action_mask > 0 {
        let action_id = u8::trailing_zeros(action_mask) as u8;

        let softirq_line = SoftIrqLine::get(action_id);
        softirq_line
            .counter
            .get()
            .unwrap()
            .add_on_cpu(CpuId::current_racy(), 1);
        softirq_line.callback.get().unwrap()();

        action_mask &= action_mask - 1;
    }
}
