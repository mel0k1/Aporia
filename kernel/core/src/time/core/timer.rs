use alloc::{
    collections::BinaryHeap,
    sync::{Arc, Weak},
    vec::Vec,
};
use core::{any::Any, time::Duration};

use ostd::sync::{LocalIrqDisabled, SpinLock, SpinLockGuard};

use super::Clock;

#[derive(Clone, Debug)]
pub(crate) enum Timeout {
    After(Duration),
    When(Duration),
}

pub(crate) struct Timer<F: ?Sized = dyn TimerFn> {
    inner: SpinLock<TimerInner>,
    timer_manager: Arc<TimerManager>,
    registered_callback: F,
}

#[derive(Default)]
struct TimerInner {
    expired_time: Option<Duration>,
    interval: Duration,
}

impl<F: TimerFn> Timer<F> {
    fn new(registered_callback: F, timer_manager: Arc<TimerManager>) -> Arc<Self> {
        Arc::new(Self {
            inner: SpinLock::new(TimerInner::default()),
            timer_manager,
            registered_callback,
        })
    }

    pub(crate) fn func(&self) -> &F {
        &self.registered_callback
    }
}

impl<F: ?Sized + TimerFn> Timer<F> {
    pub(crate) fn lock(self: &Arc<Self>) -> TimerGuard<'_, F> {
        TimerGuard {
            inner: self.inner.disable_irq().lock(),
            timer: self,
        }
    }

    pub(crate) fn timer_manager(&self) -> &Arc<TimerManager> {
        &self.timer_manager
    }
}

pub(crate) struct TimerGuard<'a, F: ?Sized = dyn TimerFn> {
    inner: SpinLockGuard<'a, TimerInner, LocalIrqDisabled>,
    timer: &'a Arc<Timer<F>>,
}

impl<F: TimerFn> TimerGuard<'_, F> {
    pub(crate) fn set_timeout(&mut self, timeout: Timeout) {
        self.set_timeout_impl(timeout, Arc::downgrade(self.timer) as _);
    }
}

impl TimerGuard<'_> {
    pub(crate) fn set_timeout(&mut self, timeout: Timeout) {
        self.set_timeout_impl(timeout, Arc::downgrade(self.timer));
    }
}

impl<F: ?Sized + TimerFn> TimerGuard<'_, F> {
    fn set_timeout_impl(&mut self, timeout: Timeout, timer_weak: Weak<Timer>) {
        let expired_time = match timeout {
            Timeout::After(timeout) => {
                let now = self.timer.timer_manager.clock.read_time();
                now + timeout
            }
            Timeout::When(timeout) => timeout,
        };

        self.inner.expired_time = Some(expired_time);

        let timer_callback = TimerCallback {
            expired_time,
            timer: timer_weak,
        };
        self.timer.timer_manager.insert(timer_callback);
    }
}

impl<F: ?Sized + TimerFn> TimerGuard<'_, F> {
    pub(crate) fn set_interval(&mut self, interval: Duration) {
        self.inner.interval = interval;
    }

    pub(crate) fn cancel(&mut self) {
        self.inner.expired_time = None;
    }

    pub(crate) fn expired_time(&self) -> Duration {
        self.inner.expired_time.unwrap_or(Duration::ZERO)
    }

    pub(crate) fn remain(&self) -> Duration {
        let now = self.timer.timer_manager.clock.read_time();
        let expired_time = self.expired_time();
        expired_time.saturating_sub(now)
    }

    pub(crate) fn interval(&self) -> Duration {
        self.inner.interval
    }
}

pub(crate) trait TimerFn: Send + Sync + Any {
    fn call(&self, guard: TimerGuard);
}

impl<T> TimerFn for T
where
    T: Fn(TimerGuard) + Send + Sync + 'static,
{
    fn call(&self, guard: TimerGuard) {
        (self)(guard)
    }
}

pub(crate) struct TimerManager {
    clock: Arc<dyn Clock>,
    timer_callbacks: SpinLock<BinaryHeap<TimerCallback>>,
}

impl TimerManager {
    pub(crate) fn new(clock: Arc<dyn Clock>) -> Arc<Self> {
        Arc::new(Self {
            clock,
            timer_callbacks: SpinLock::new(BinaryHeap::new()),
        })
    }

    pub(crate) fn clock(&self) -> &Arc<dyn Clock> {
        &self.clock
    }

    pub(crate) fn is_expired_timeout(&self, timeout: &Timeout) -> bool {
        match timeout {
            Timeout::After(duration) => *duration == Duration::ZERO,
            Timeout::When(duration) => {
                let now = self.clock.read_time();
                now >= *duration
            }
        }
    }

    fn insert(&self, timer_callback: TimerCallback) {
        self.timer_callbacks
            .disable_irq()
            .lock()
            .push(timer_callback);
    }

    pub(crate) fn process_expired_timers(&self) {
        let callbacks = {
            let mut timeout_list = self.timer_callbacks.disable_irq().lock();
            if timeout_list.is_empty() {
                return;
            }

            let mut callbacks = Vec::new();
            let current_time = self.clock.read_time();
            while let Some(t) = timeout_list.peek() {
                if t.expired_time <= current_time {
                    callbacks.push(timeout_list.pop().unwrap());
                } else {
                    break;
                }
            }
            callbacks
        };

        for callback in callbacks {
            callback.call();
        }
    }

    pub(crate) fn create_timer<F: TimerFn>(self: &Arc<Self>, function: F) -> Arc<Timer<F>> {
        Timer::new(function, self.clone())
    }
}

struct TimerCallback {
    expired_time: Duration,
    timer: Weak<Timer>,
}

impl TimerCallback {
    fn call(&self) {
        let Some(timer) = self.timer.upgrade() else {
            return;
        };

        let mut timer_guard = timer.lock();

        if timer_guard.inner.expired_time != Some(self.expired_time) {
            return;
        }

        let interval = timer_guard.interval();
        if interval != Duration::ZERO {
            timer_guard.set_timeout(Timeout::After(interval));
        } else {
            timer_guard.cancel();
        }

        timer.registered_callback.call(timer_guard);
    }
}

impl PartialEq for TimerCallback {
    fn eq(&self, other: &Self) -> bool {
        self.expired_time == other.expired_time
    }
}

impl Eq for TimerCallback {}

impl PartialOrd for TimerCallback {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TimerCallback {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.expired_time.cmp(&other.expired_time).reverse()
    }
}
