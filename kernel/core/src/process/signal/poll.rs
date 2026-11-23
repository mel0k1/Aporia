use core::{
    sync::atomic::{AtomicIsize, Ordering},
    time::Duration,
};

use ostd::{
    sync::{Waiter, Waker},
    task::Task,
};

use crate::{
    events::{IoEvents, Observer, SyncSubject},
    prelude::*,
    time::wait::TimeoutExt,
};

#[derive(Clone)]
pub(crate) struct Pollee {
    inner: Arc<PolleeInner>,
}

const INV_STATE: isize = -1;

struct PolleeInner {
    subject: SyncSubject<IoEvents, IoEvents>,
    state: AtomicIsize,
}

impl Default for Pollee {
    fn default() -> Self {
        Self::new()
    }
}

impl Pollee {
    pub(crate) fn new() -> Self {
        let inner = PolleeInner {
            subject: SyncSubject::new(),
            state: AtomicIsize::new(INV_STATE),
        };
        Self {
            inner: Arc::new(inner),
        }
    }

    pub(crate) fn poll_with<F>(
        &self,
        mask: IoEvents,
        poller: Option<&mut PollHandle>,
        check: F,
    ) -> IoEvents
    where
        F: FnOnce() -> IoEvents,
    {
        let mask = mask | IoEvents::ALWAYS_POLL;

        if let Some(poller) = poller {
            self.register_poller(poller, mask);
        }

        let events = self.inner.state.load(Ordering::Acquire);
        if events >= 0 {
            return IoEvents::from_bits_truncate(events as _) & mask;
        }

        if events != INV_STATE {
            return check() & mask;
        }

        const {
            use ostd::mm::KERNEL_VADDR_RANGE;
            assert!((KERNEL_VADDR_RANGE.start as isize) < 0);
        }
        let task_ptr = Task::current().unwrap().as_ref() as *const _ as isize;

        let _ = self.inner.state.swap(task_ptr, Ordering::Acquire);

        let new_events = check();

        let _ = self.inner.state.compare_exchange_weak(
            task_ptr,
            new_events.bits() as _,
            Ordering::Release,
            Ordering::Relaxed,
        );

        new_events & mask
    }

    pub(crate) fn register_poller(&self, poller: &mut PollHandle, mask: IoEvents) {
        self.inner
            .subject
            .register_observer(poller.observer.clone(), mask);

        poller.pollees.push(Arc::downgrade(&self.inner));
    }

    pub(crate) fn notify(&self, events: IoEvents) {
        self.invalidate();

        self.inner.subject.notify_observers(&events);
    }

    pub(crate) fn invalidate(&self) {
        self.inner.state.store(INV_STATE, Ordering::Release);
    }
}

pub struct PollHandle {
    observer: Weak<dyn Observer<IoEvents>>,
    pollees: Vec<Weak<PolleeInner>>,
}

impl PollHandle {
    pub(crate) fn new(observer: Weak<dyn Observer<IoEvents>>) -> Self {
        Self {
            observer,
            pollees: Vec::new(),
        }
    }

    pub(crate) fn reset(&mut self) {
        let observer = &self.observer;

        self.pollees
            .iter()
            .filter_map(Weak::upgrade)
            .for_each(|pollee| {
                pollee.subject.unregister_observer(observer);
            });
    }
}

impl Drop for PollHandle {
    fn drop(&mut self) {
        self.reset();
    }
}

pub(crate) struct PollAdaptor<O> {
    observer: Arc<O>,
    inner: PollHandle,
}

impl<O: Observer<IoEvents> + 'static> PollAdaptor<O> {
    pub(crate) fn with_observer(observer: O) -> Self {
        let observer = Arc::new(observer);
        let inner = PollHandle::new(Arc::downgrade(&observer) as _);

        Self { observer, inner }
    }
}

impl<O> PollAdaptor<O> {
    #[expect(dead_code, reason = "Keep this `Arc` to avoid dropping the observer")]
    pub(crate) fn observer(&self) -> &Arc<O> {
        &self.observer
    }

    pub(crate) fn as_handle_mut(&mut self) -> &mut PollHandle {
        &mut self.inner
    }
}

pub(crate) struct Poller {
    poller: PollHandle,
    waiter: Waiter,
    timeout: TimeoutExt<'static>,
}

impl Poller {
    pub(crate) fn new(timeout: Option<&Duration>) -> Self {
        let (waiter, waker) = Waiter::new_pair();

        let mut timeout_ext = TimeoutExt::from(timeout);
        timeout_ext.freeze();

        Self {
            poller: PollHandle::new(Arc::downgrade(&waker) as Weak<_>),
            waiter,
            timeout: timeout_ext,
        }
    }

    pub(crate) fn as_handle_mut(&mut self) -> &mut PollHandle {
        &mut self.poller
    }

    pub(crate) fn wait(&self) -> Result<()> {
        self.waiter.pause_timeout(&self.timeout)
    }
}

impl Observer<IoEvents> for Waker {
    fn on_events(&self, _events: &IoEvents) {
        self.wake_up();
    }
}

pub trait Pollable {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents;

    #[track_caller]
    fn wait_events<F, R>(
        &self,
        mask: IoEvents,
        timeout: Option<&Duration>,
        mut try_op: F,
    ) -> Result<R>
    where
        Self: Sized,
        F: FnMut() -> Result<R>,
    {
        match try_op() {
            Err(err) if err.error() == Errno::EAGAIN => (),
            result => return result,
        }

        if timeout.is_some_and(|duration| duration.is_zero()) {
            return Err(errno!(ETIME, "the timeout expired"));
        }

        let mut poller = Poller::new(timeout);
        if self.poll(mask, Some(poller.as_handle_mut())).is_empty() {
            poller.wait()?;
        }

        loop {
            match try_op() {
                Err(err) if err.error() == Errno::EAGAIN => (),
                result => return result,
            };

            poller.wait()?;
        }
    }
}

#[cfg(ktest)]
mod test {
    use ostd::prelude::*;

    use super::*;

    #[ktest]
    fn notify_before() {
        let pollee = Pollee::new();

        pollee.notify(IoEvents::OUT);

        assert_eq!(
            pollee.poll_with(IoEvents::all(), None, || IoEvents::IN),
            IoEvents::IN
        );

        assert_eq!(
            pollee.poll_with(IoEvents::all(), None, || IoEvents::OUT),
            IoEvents::IN
        );
    }

    #[ktest]
    fn notify_middle() {
        let pollee = Pollee::new();

        assert_eq!(
            pollee.poll_with(IoEvents::all(), None, || {
                pollee.notify(IoEvents::OUT);
                IoEvents::IN
            }),
            IoEvents::IN
        );

        assert_eq!(
            pollee.poll_with(IoEvents::all(), None, || IoEvents::OUT),
            IoEvents::OUT
        );
    }

    #[ktest]
    fn notify_after() {
        let pollee = Pollee::new();

        assert_eq!(
            pollee.poll_with(IoEvents::all(), None, || IoEvents::IN),
            IoEvents::IN
        );

        pollee.notify(IoEvents::OUT);

        assert_eq!(
            pollee.poll_with(IoEvents::all(), None, || IoEvents::OUT),
            IoEvents::OUT
        );
    }

    #[ktest]
    fn nested_notify_before() {
        let pollee = Pollee::new();

        pollee.notify(IoEvents::OUT);

        assert_eq!(
            pollee.poll_with(IoEvents::all(), None, || {
                assert_eq!(
                    pollee.poll_with(IoEvents::all(), None, || IoEvents::OUT),
                    IoEvents::OUT
                );
                IoEvents::IN
            }),
            IoEvents::IN
        );

        assert_eq!(
            pollee.poll_with(IoEvents::all(), None, || IoEvents::OUT),
            IoEvents::IN
        );
    }

    #[ktest]
    fn nested_notify_between() {
        let pollee = Pollee::new();

        assert_eq!(
            pollee.poll_with(IoEvents::all(), None, || {
                pollee.notify(IoEvents::OUT);
                assert_eq!(
                    pollee.poll_with(IoEvents::all(), None, || IoEvents::OUT),
                    IoEvents::OUT
                );
                IoEvents::IN
            }),
            IoEvents::IN
        );

        assert_eq!(
            pollee.poll_with(IoEvents::all(), None, || IoEvents::OUT),
            IoEvents::OUT
        );
    }

    #[ktest]
    fn nested_notify_inside() {
        let pollee = Pollee::new();

        assert_eq!(
            pollee.poll_with(IoEvents::all(), None, || {
                assert_eq!(
                    pollee.poll_with(IoEvents::all(), None, || {
                        pollee.notify(IoEvents::OUT);
                        IoEvents::OUT
                    }),
                    IoEvents::OUT
                );
                IoEvents::IN
            }),
            IoEvents::IN
        );

        assert_eq!(
            pollee.poll_with(IoEvents::all(), None, || IoEvents::OUT),
            IoEvents::OUT,
        );
    }
}
