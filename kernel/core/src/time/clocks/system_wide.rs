use alloc::sync::Arc;
use core::time::Duration;

use aster_time::{read_monotonic_time, read_start_time, read_system_time};
use ostd::{cpu::PinCurrentCpu, cpu_local, sync::SpinLock, task::disable_preempt, timer::Jiffies};
use paste::paste;
use spin::Once;

use crate::time::{self, Clock, timer::TimerManager};

pub(crate) struct JiffiesClock {
    _private: (),
}

pub(crate) struct RealTimeClock {
    _private: (),
}

impl RealTimeClock {
    pub(crate) fn get() -> &'static Arc<RealTimeClock> {
        CLOCK_REALTIME_INSTANCE.get().unwrap()
    }

    pub(crate) fn timer_manager() -> &'static Arc<TimerManager> {
        let preempt_guard = disable_preempt();
        CLOCK_REALTIME_MANAGER
            .get_on_cpu(preempt_guard.current_cpu())
            .get()
            .unwrap()
    }
}

pub(crate) struct MonotonicClock {
    _private: (),
}

impl MonotonicClock {
    pub(crate) fn get() -> &'static Arc<MonotonicClock> {
        CLOCK_MONOTONIC_INSTANCE.get().unwrap()
    }

    pub(crate) fn timer_manager() -> &'static Arc<TimerManager> {
        let preempt_guard = disable_preempt();
        CLOCK_MONOTONIC_MANAGER
            .get_on_cpu(preempt_guard.current_cpu())
            .get()
            .unwrap()
    }
}

pub(crate) struct RealTimeCoarseClock {
    _private: (),
}

impl RealTimeCoarseClock {
    pub(crate) fn get() -> &'static Arc<RealTimeCoarseClock> {
        CLOCK_REALTIME_COARSE_INSTANCE.get().unwrap()
    }

    fn current_ref() -> &'static Once<SpinLock<Duration>> {
        static CURRENT: Once<SpinLock<Duration>> = Once::new();

        &CURRENT
    }
}

pub(crate) struct MonotonicCoarseClock {
    _private: (),
}

impl MonotonicCoarseClock {
    pub(crate) fn get() -> &'static Arc<MonotonicCoarseClock> {
        CLOCK_MONOTONIC_COARSE_INSTANCE.get().unwrap()
    }
}

pub(crate) struct MonotonicRawClock {
    _private: (),
}

impl MonotonicRawClock {
    pub(crate) fn get() -> &'static Arc<MonotonicRawClock> {
        CLOCK_MONOTONIC_RAW_INSTANCE.get().unwrap()
    }
}

pub(crate) struct BootTimeClock {
    _private: (),
}

impl BootTimeClock {
    pub(crate) fn get() -> &'static Arc<BootTimeClock> {
        CLOCK_BOOTTIME_INSTANCE.get().unwrap()
    }

    pub(crate) fn timer_manager() -> &'static Arc<TimerManager> {
        let preempt_guard = disable_preempt();
        CLOCK_BOOTTIME_MANAGER
            .get_on_cpu(preempt_guard.current_cpu())
            .get()
            .unwrap()
    }
}

impl Clock for JiffiesClock {
    fn read_time(&self) -> Duration {
        Jiffies::elapsed().as_duration()
    }
}

impl Clock for RealTimeClock {
    fn read_time(&self) -> Duration {
        read_system_time().since_epoch()
    }
}

impl Clock for MonotonicClock {
    fn read_time(&self) -> Duration {
        read_monotonic_time()
    }
}

impl Clock for RealTimeCoarseClock {
    fn read_time(&self) -> Duration {
        *Self::current_ref().get().unwrap().disable_irq().lock()
    }
}

impl Clock for MonotonicCoarseClock {
    fn read_time(&self) -> Duration {
        RealTimeCoarseClock::get().read_time() - read_start_time().since_epoch()
    }
}

impl Clock for MonotonicRawClock {
    fn read_time(&self) -> Duration {
        read_monotonic_time()
    }
}

impl Clock for BootTimeClock {
    fn read_time(&self) -> Duration {
        read_monotonic_time()
    }
}

macro_rules! define_system_clocks {
    ($($clock_id:ident => $clock_type:ident,)*) => {
        $(
            paste! {
                pub(crate) static [<$clock_id _INSTANCE>]: Once<Arc<$clock_type>> = Once::new();
            }
        )*

        fn _init_system_wide_clocks() {
            $(
                let clock = Arc::new(
                    $clock_type {
                        _private: (),
                    }
                );
                paste! {
                    [<$clock_id _INSTANCE>].call_once(|| clock.clone());
                }
            )*
        }
    }
}

macro_rules! define_timer_managers {
    ($($clock_id:ident,)*) => {
        $(
            paste! {
                cpu_local! {
                    pub(crate) static [<$clock_id _MANAGER>]: Once<Arc<TimerManager>> = Once::new();
                }
            }
        )*

        fn _init_system_wide_timer_managers() {
            $(
                let clock = paste! {[<$clock_id _INSTANCE>].get().unwrap().clone()};
                let timer_manager = TimerManager::new(clock);
                for cpu in ostd::cpu::all_cpus() {
                    paste! {
                        [<$clock_id _MANAGER>].get_on_cpu(cpu).call_once(|| timer_manager.clone());
                    }
                }
                let callback = || {
                    let preempt_guard = ostd::task::disable_preempt();
                    let cpu = preempt_guard.current_cpu();
                    paste! {
                        [<$clock_id _MANAGER>].get_on_cpu(cpu).get().unwrap().process_expired_timers();
                    }
                };
                time::softirq::register_callback(callback);
            )*
        }
    }
}

define_system_clocks! {
    CLOCK_REALTIME          => RealTimeClock,
    CLOCK_REALTIME_COARSE   => RealTimeCoarseClock,
    CLOCK_MONOTONIC         => MonotonicClock,
    CLOCK_MONOTONIC_COARSE  => MonotonicCoarseClock,
    CLOCK_MONOTONIC_RAW     => MonotonicRawClock,
    CLOCK_BOOTTIME          => BootTimeClock,
}

define_timer_managers![CLOCK_REALTIME, CLOCK_MONOTONIC, CLOCK_BOOTTIME,];

fn init_system_wide_clocks() {
    _init_system_wide_clocks();
}

fn init_system_wide_timer_managers() {
    _init_system_wide_timer_managers();
}

pub(crate) static JIFFIES_TIMER_MANAGER: Once<Arc<TimerManager>> = Once::new();

fn init_jiffies_clock_manager() {
    let jiffies_clock = JiffiesClock { _private: () };
    let jiffies_timer_manager = TimerManager::new(Arc::new(jiffies_clock));
    JIFFIES_TIMER_MANAGER.call_once(|| jiffies_timer_manager);

    let callback = || {
        JIFFIES_TIMER_MANAGER
            .get()
            .unwrap()
            .process_expired_timers();
    };
    time::softirq::register_callback(callback);
}

fn update_coarse_clock() {
    let real_time = RealTimeClock::get().read_time();
    let current = RealTimeCoarseClock::current_ref().get().unwrap();
    *current.disable_irq().lock() = real_time;
}

fn init_coarse_clock() {
    let real_time = RealTimeClock::get().read_time();
    RealTimeCoarseClock::current_ref().call_once(|| SpinLock::new(real_time));
    time::softirq::register_callback(update_coarse_clock);
}

pub(super) fn init() {
    init_system_wide_clocks();
    init_system_wide_timer_managers();
    init_jiffies_clock_manager();
    init_coarse_clock();
}

#[cfg(ktest)]
pub(crate) fn init_for_ktest() {
    for cpu in ostd::cpu::all_cpus() {
        CLOCK_REALTIME_MANAGER.get_on_cpu(cpu).call_once(|| {
            let clock = RealTimeClock { _private: () };
            TimerManager::new(Arc::new(clock))
        });
    }
    CLOCK_REALTIME_COARSE_INSTANCE.call_once(|| Arc::new(RealTimeCoarseClock { _private: () }));
    RealTimeCoarseClock::current_ref().call_once(|| SpinLock::new(Duration::from_secs(0)));
    JIFFIES_TIMER_MANAGER.call_once(|| {
        let clock = JiffiesClock { _private: () };
        TimerManager::new(Arc::new(clock))
    });
}
