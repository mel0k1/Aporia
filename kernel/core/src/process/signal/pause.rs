use ostd::sync::{WaitQueue, Waiter};

use super::sig_mask::SigMask;
use crate::{
    prelude::*,
    process::{
        posix_thread::{AsPosixThread, ContextPthreadAdminApi},
        signal::HandlePendingSignal,
    },
    thread::AsThread,
    time::{
        timer::TimerGuard,
        wait::{ManagedTimeout, TimeoutExt},
    },
};

pub(crate) trait Pause: WaitTimeout {
    #[track_caller]
    fn pause_until<F, R>(&self, cond: F) -> Result<R>
    where
        F: FnMut() -> Option<R>,
    {
        self.pause_until_or_timeout_impl(cond, None, PauseReason::Sleep)
    }

    #[track_caller]
    fn pause_until_by<F, R>(&self, cond: F, reason: PauseReason) -> Result<R>
    where
        F: FnMut() -> Option<R>,
    {
        self.pause_until_or_timeout_impl(cond, None, reason)
    }

    #[track_caller]
    fn pause_until_or_timeout<'a, F, T, R>(&self, mut cond: F, timeout: T) -> Result<R>
    where
        F: FnMut() -> Option<R>,
        T: Into<TimeoutExt<'a>>,
    {
        let timeout = timeout.into();
        let timeout_inner = match timeout.check_expired() {
            Ok(inner) => inner,
            Err(err) => return cond().ok_or(err),
        };

        self.pause_until_or_timeout_impl(cond, timeout_inner, PauseReason::Sleep)
    }

    #[doc(hidden)]
    #[track_caller]
    fn pause_until_or_timeout_impl<F, R>(
        &self,
        cond: F,
        timeout: Option<&ManagedTimeout>,
        reason: PauseReason,
    ) -> Result<R>
    where
        F: FnMut() -> Option<R>;

    #[track_caller]
    fn pause_timeout(&self, timeout: &TimeoutExt<'_>) -> Result<()>;
}

impl Pause for Waiter {
    fn pause_until_or_timeout_impl<F, R>(
        &self,
        cond: F,
        timeout: Option<&ManagedTimeout>,
        reason: PauseReason,
    ) -> Result<R>
    where
        F: FnMut() -> Option<R>,
    {
        let Some(posix_thread) = self
            .task()
            .as_thread()
            .and_then(|thread| thread.as_posix_thread())
        else {
            return self.wait_until_or_timeout_cancelled(cond, || Ok(()), timeout);
        };

        let cancel_cond = || {
            let has_pending = match reason {
                PauseReason::StopByPtrace => posix_thread.has_pending_sigkill(),
                _ => posix_thread.has_pending(),
            };

            if has_pending {
                return Err(errno!(
                    EINTR,
                    "the current thread is interrupted by a signal"
                ));
            }
            Ok(())
        };

        posix_thread.set_signalled_waker(self.waker(), reason);
        let res = self.wait_until_or_timeout_cancelled(cond, cancel_cond, timeout);
        posix_thread.clear_signalled_waker();

        res
    }

    fn pause_timeout(&self, timeout: &TimeoutExt<'_>) -> Result<()> {
        let timer = timeout.check_expired()?.map(|timeout| {
            let waker = self.waker();
            timeout.create_timer(move |_guard: TimerGuard| {
                waker.wake_up();
            })
        });

        let posix_thread_opt = self
            .task()
            .as_thread()
            .and_then(|thread| thread.as_posix_thread());

        if let Some(posix_thread) = posix_thread_opt {
            posix_thread.set_signalled_waker(self.waker(), PauseReason::Sleep);
            if posix_thread.has_pending() {
                posix_thread.clear_signalled_waker();
                return Err(errno!(
                    EINTR,
                    "the current thread is interrupted by a signal"
                ));
            }

            self.wait();

            posix_thread.clear_signalled_waker();
        } else {
            self.wait();
        }

        if let Some(timer) = timer {
            let mut timer_guard = timer.lock();
            if timer_guard.remain().is_zero() {
                return Err(errno!(ETIME, "the time limit is reached"));
            }
            timer_guard.cancel();
        }

        if posix_thread_opt
            .as_ref()
            .is_some_and(|posix_thread| posix_thread.has_pending())
        {
            return Err(errno!(
                EINTR,
                "the current thread is interrupted by a signal"
            ));
        }

        Ok(())
    }
}

impl Pause for WaitQueue {
    fn pause_until_or_timeout_impl<F, R>(
        &self,
        mut cond: F,
        timeout: Option<&ManagedTimeout>,
        reason: PauseReason,
    ) -> Result<R>
    where
        F: FnMut() -> Option<R>,
    {
        if let Some(res) = cond() {
            return Ok(res);
        }

        let (waiter, _) = Waiter::new_pair();
        let cond = || {
            self.enqueue(waiter.waker());
            cond()
        };
        waiter.pause_until_or_timeout_impl(cond, timeout, reason)
    }

    fn pause_timeout(&self, _timeout: &TimeoutExt<'_>) -> Result<()> {
        panic!("`pause_timeout` can only be used on `Waiter`");
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum PauseReason {
    Sleep,
    StopBySignal,
    StopByPtrace,
}

pub(crate) fn with_sigmask_changed<R>(
    ctx: &Context,
    mask_op: impl FnOnce(SigMask) -> SigMask,
    operate: impl FnOnce() -> R,
) -> R {
    let old_mask = ctx.posix_thread.sig_mask();
    ctx.set_sig_mask(mask_op(old_mask));

    let res = operate();

    ctx.set_sig_mask(old_mask);

    res
}

#[cfg(ktest)]
mod test {
    use core::sync::atomic::{AtomicBool, Ordering};

    use ostd::prelude::*;

    use super::*;
    use crate::thread::{Thread, kernel_thread::ThreadOptions};

    #[ktest]
    fn waiter_pause() {
        let wait_queue = Arc::new(WaitQueue::new());
        let wait_queue_cloned = wait_queue.clone();

        let boolean = Arc::new(AtomicBool::new(false));
        let boolean_cloned = boolean.clone();

        let thread = ThreadOptions::new(move || {
            Thread::yield_now();

            boolean_cloned.store(true, Ordering::Relaxed);
            wait_queue_cloned.wake_all();
        })
        .spawn();

        wait_queue
            .pause_until(|| boolean.load(Ordering::Relaxed).then_some(()))
            .unwrap();

        thread.join();
    }
}
