use alloc::collections::btree_map::Values;

use super::{BOOTSTRAP_PGID, BOOTSTRAP_SID, Pgid, Pid, Process, Session};
use crate::{prelude::*, process::signal::signals::Signal};

pub(crate) struct ProcessGroup {
    pgid: Pgid,
    session: Arc<Session>,
    inner: Mutex<Inner>,
}

struct Inner {
    processes: BTreeMap<Pid, Weak<Process>>,
}

impl ProcessGroup {
    pub(super) fn new(process: &Arc<Process>, session: Arc<Session>) -> Arc<Self> {
        Self::new_with(process.pid(), process, session)
    }

    pub(super) fn new_bootstrap(process: &Arc<Process>, session: Arc<Session>) -> Arc<Self> {
        debug_assert!(process.is_init_process());
        debug_assert_eq!(session.sid(), BOOTSTRAP_SID);

        Self::new_with(BOOTSTRAP_PGID, process, session)
    }

    fn new_with(pgid: Pgid, process: &Arc<Process>, session: Arc<Session>) -> Arc<Self> {
        let inner = {
            let mut processes = BTreeMap::new();
            processes.insert(process.pid(), Arc::downgrade(process));
            Inner { processes }
        };

        Arc::new(ProcessGroup {
            pgid,
            session,
            inner: Mutex::new(inner),
        })
    }

    pub(crate) fn pgid(&self) -> Pgid {
        self.pgid
    }

    pub(crate) fn session(&self) -> &Arc<Session> {
        &self.session
    }

    pub(crate) fn lock(&self) -> ProcessGroupGuard<'_> {
        ProcessGroupGuard {
            inner: self.inner.lock(),
        }
    }

    pub(crate) fn broadcast_signal(&self, signal: impl Signal + Clone + 'static) {
        for process in self
            .inner
            .lock()
            .processes
            .values()
            .filter_map(Weak::upgrade)
        {
            process.enqueue_signal(Box::new(signal.clone()));
        }
    }
}

#[clippy::has_significant_drop]
#[must_use]
pub(crate) struct ProcessGroupGuard<'a> {
    inner: MutexGuard<'a, Inner>,
}

impl ProcessGroupGuard<'_> {
    pub(crate) fn iter(&self) -> ProcessGroupIter<'_> {
        ProcessGroupIter {
            inner: self.inner.processes.values(),
        }
    }

    pub(in crate::process) fn insert_process(&mut self, process: &Arc<Process>) {
        let old_process = self
            .inner
            .processes
            .insert(process.pid(), Arc::downgrade(process));
        debug_assert!(old_process.is_none());
    }

    pub(in crate::process) fn remove_process(&mut self, pid: &Pid) {
        let process = self.inner.processes.remove(pid);
        debug_assert!(process.is_some());
    }

    pub(in crate::process) fn is_empty(&self) -> bool {
        self.inner.processes.is_empty()
    }
}

pub(crate) struct ProcessGroupIter<'a> {
    inner: Values<'a, Pid, Weak<Process>>,
}

impl<'a> Iterator for ProcessGroupIter<'a> {
    type Item = Arc<Process>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.by_ref().find_map(Weak::upgrade)
    }
}
