use ostd::sync::{LocalIrqDisabled, WaitQueue};

use super::{ProcessGroup, Session};
use crate::prelude::*;

pub(crate) struct JobControl {
    inner: SpinLock<Inner, LocalIrqDisabled>,
    wait_queue: WaitQueue,
}

#[derive(Default)]
struct Inner {
    session: Weak<Session>,
    foreground: Weak<ProcessGroup>,
}

impl JobControl {
    pub(crate) fn new() -> Self {
        Self {
            inner: SpinLock::new(Inner::default()),
            wait_queue: WaitQueue::new(),
        }
    }

    pub(super) fn session(&self) -> Option<Arc<Session>> {
        self.inner.lock().session.upgrade()
    }

    pub(super) fn set_session(
        &self,
        session: &Arc<Session>,
        foreground: &Arc<ProcessGroup>,
    ) -> Result<()> {
        let mut inner = self.inner.lock();

        if inner.session.upgrade().is_some() {
            return Err(errno!(
                EPERM,
                "the terminal is already a controlling terminal of another session"
            ));
        }

        *inner = Inner {
            session: Arc::downgrade(session),
            foreground: Arc::downgrade(foreground),
        };

        Ok(())
    }

    pub(super) fn unset_session(&self) -> Option<Arc<ProcessGroup>> {
        let mut inner = self.inner.lock();

        debug_assert!(inner.session.upgrade().is_some());

        let foreground = inner.foreground.upgrade();
        *inner = Inner::default();

        foreground
    }

    pub(crate) fn foreground(&self) -> Option<Arc<ProcessGroup>> {
        self.inner.lock().foreground.upgrade()
    }

    pub(super) fn set_foreground(&self, process_group: &Arc<ProcessGroup>) {
        let mut inner = self.inner.lock();

        debug_assert!(Arc::ptr_eq(
            process_group.session(),
            &inner.session.upgrade().unwrap()
        ));
        inner.foreground = Arc::downgrade(process_group);

        self.wait_queue.wake_all();
    }

    pub(crate) fn wait_until_in_foreground(&self) -> Result<()> {
        let current = current!();

        self.wait_queue.pause_until(|| {
            let process_group_mut = current.process_group.lock();
            let process_group = process_group_mut.as_ref().unwrap();
            let session = process_group.session();

            let inner = self.inner.lock();
            if !inner
                .session
                .upgrade()
                .is_some_and(|terminal_session| Arc::ptr_eq(&terminal_session, session))
            {
                return Some(());
            }

            inner
                .foreground
                .upgrade()
                .is_some_and(|terminal_foregroup| Arc::ptr_eq(&terminal_foregroup, process_group))
                .then_some(())
        })
    }
}

impl Default for JobControl {
    fn default() -> Self {
        Self::new()
    }
}
