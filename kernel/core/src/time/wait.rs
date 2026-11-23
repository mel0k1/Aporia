use core::time::Duration;

use ostd::sync::{WaitQueue, Waiter};

use super::{Timer, TimerManager, clocks::JIFFIES_TIMER_MANAGER, timer::Timeout};
use crate::{
    prelude::*,
    time::timer::{TimerFn, TimerGuard},
};

pub(crate) trait WaitTimeout {
    #[track_caller]
    fn wait_until_or_timeout<'a, F, T, R>(&self, mut cond: F, timeout: T) -> Result<R>
    where
        F: FnMut() -> Option<R>,
        T: Into<TimeoutExt<'a>>,
    {
        let timeout = timeout.into();
        let timeout_inner = match timeout.check_expired() {
            Ok(inner) => inner,
            Err(err) => return cond().ok_or(err),
        };

        self.wait_until_or_timeout_cancelled(cond, || Ok(()), timeout_inner)
    }

    #[doc(hidden)]
    #[track_caller]
    fn wait_until_or_timeout_cancelled<F, R, FCancel>(
        &self,
        cond: F,
        cancel_cond: FCancel,
        timeout: Option<&ManagedTimeout>,
    ) -> Result<R>
    where
        F: FnMut() -> Option<R>,
        FCancel: Fn() -> Result<()>;
}

pub(crate) enum TimeoutExt<'a> {
    Never,
    At(ManagedTimeout<'a>),
}

impl<'a> TimeoutExt<'a> {
    pub(crate) fn check_expired(&self) -> Result<Option<&ManagedTimeout<'a>>> {
        match self {
            TimeoutExt::At(inner) if inner.is_expired() => {
                Err(errno!(ETIME, "the time limit is reached"))
            }
            TimeoutExt::At(inner) => Ok(Some(inner)),
            TimeoutExt::Never => Ok(None),
        }
    }

    pub(crate) fn freeze(&mut self) {
        match self {
            Self::Never => (),
            Self::At(timeout) => timeout.freeze(),
        }
    }
}

impl From<&Duration> for TimeoutExt<'_> {
    fn from(value: &Duration) -> Self {
        Self::At(ManagedTimeout::new(*value))
    }
}

impl From<Option<&Duration>> for TimeoutExt<'_> {
    fn from(value: Option<&Duration>) -> Self {
        match value {
            Some(duration) => duration.into(),
            None => Self::Never,
        }
    }
}

impl<'a> From<ManagedTimeout<'a>> for TimeoutExt<'a> {
    fn from(value: ManagedTimeout<'a>) -> Self {
        Self::At(value)
    }
}

impl<'a> From<Option<ManagedTimeout<'a>>> for TimeoutExt<'a> {
    fn from(value: Option<ManagedTimeout<'a>>) -> Self {
        match value {
            Some(timeout) => timeout.into(),
            None => Self::Never,
        }
    }
}

pub(crate) struct ManagedTimeout<'a> {
    timeout: Timeout,
    manager: &'a Arc<TimerManager>,
}

impl<'a> ManagedTimeout<'a> {
    pub(crate) fn new(timeout: Duration) -> Self {
        let timeout = Timeout::After(timeout);
        let manager = JIFFIES_TIMER_MANAGER.get().unwrap();
        Self::new_with_manager(timeout, manager)
    }

    pub(crate) const fn new_with_manager(timeout: Timeout, manager: &'a Arc<TimerManager>) -> Self {
        Self { timeout, manager }
    }

    pub(crate) fn is_expired(&self) -> bool {
        self.manager.is_expired_timeout(&self.timeout)
    }

    pub(crate) fn freeze(&mut self) {
        self.timeout = match self.timeout {
            Timeout::When(instant) => Timeout::When(instant),
            Timeout::After(duration) => Timeout::When(self.manager.clock().read_time() + duration),
        }
    }

    pub(crate) fn create_timer<F: TimerFn>(&self, callback: F) -> Arc<Timer<F>> {
        let timer = self.manager.create_timer(callback);
        timer.lock().set_timeout(self.timeout.clone());
        timer
    }
}

impl WaitTimeout for Waiter {
    fn wait_until_or_timeout_cancelled<F, R, FCancel>(
        &self,
        cond: F,
        cancel_cond: FCancel,
        timeout: Option<&ManagedTimeout>,
    ) -> Result<R>
    where
        F: FnMut() -> Option<R>,
        FCancel: Fn() -> Result<()>,
    {
        let timer = timeout.map(|timeout| {
            let waker = self.waker();
            timeout.create_timer(move |_guard: TimerGuard| {
                waker.wake_up();
            })
        });

        let cancel_cond = {
            let timer = timer.clone();

            move || {
                if timer
                    .as_ref()
                    .is_some_and(|timer| timer.lock().remain() == Duration::ZERO)
                {
                    return Err(errno!(ETIME, "the time limit is reached"));
                }

                cancel_cond()
            }
        };

        let res = self.wait_until_or_cancelled(cond, cancel_cond);

        if let Some(timer) = timer
            && !res
                .as_ref()
                .is_err_and(|e: &Error| e.error() == Errno::ETIME)
        {
            timer.lock().cancel();
        }

        res
    }
}

impl WaitTimeout for WaitQueue {
    fn wait_until_or_timeout_cancelled<F, R, FCancel>(
        &self,
        mut cond: F,
        cancel_cond: FCancel,
        timeout: Option<&ManagedTimeout>,
    ) -> Result<R>
    where
        F: FnMut() -> Option<R>,
        FCancel: Fn() -> Result<()>,
    {
        if let Some(res) = cond() {
            return Ok(res);
        }

        let (waiter, _) = Waiter::new_pair();
        let cond = || {
            self.enqueue(waiter.waker());
            cond()
        };
        waiter.wait_until_or_timeout_cancelled(cond, cancel_cond, timeout)
    }
}
