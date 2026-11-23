use super::{BOOTSTRAP_SID, Pgid, Process, ProcessGroup, Sid, Terminal};
use crate::prelude::*;

pub(crate) struct Session {
    sid: Sid,
    inner: Mutex<Inner>,
}

struct Inner {
    process_groups: BTreeMap<Pgid, Weak<ProcessGroup>>,
    terminal: Option<Arc<dyn Terminal>>,
}

impl Session {
    pub(super) fn new_pair(process: &Arc<Process>) -> (Arc<Self>, Arc<ProcessGroup>) {
        let session = Self::new_empty(process.pid());
        let process_group = ProcessGroup::new(process, session.clone());
        session.lock().insert_process_group(&process_group);

        (session, process_group)
    }

    pub(super) fn new_bootstrap_pair(process: &Arc<Process>) -> (Arc<Self>, Arc<ProcessGroup>) {
        debug_assert!(process.is_init_process());

        let session = Self::new_empty(BOOTSTRAP_SID);
        let process_group = ProcessGroup::new_bootstrap(process, session.clone());
        session.lock().insert_process_group(&process_group);

        (session, process_group)
    }

    fn new_empty(sid: Sid) -> Arc<Self> {
        Arc::new(Self {
            sid,
            inner: Mutex::new(Inner {
                process_groups: BTreeMap::new(),
                terminal: None,
            }),
        })
    }

    pub(crate) fn sid(&self) -> Sid {
        self.sid
    }

    pub(super) fn is_leader(&self, process: &Process) -> bool {
        self.sid == process.pid()
    }

    pub(crate) fn lock(&self) -> SessionGuard<'_> {
        SessionGuard {
            inner: self.inner.lock(),
        }
    }
}

#[clippy::has_significant_drop]
#[must_use]
pub(crate) struct SessionGuard<'a> {
    inner: MutexGuard<'a, Inner>,
}

impl SessionGuard<'_> {
    pub(super) fn set_terminal(&mut self, terminal: Option<Arc<dyn Terminal>>) {
        self.inner.terminal = terminal;
    }

    pub(crate) fn terminal(&self) -> Option<&Arc<dyn Terminal>> {
        self.inner.terminal.as_ref()
    }

    pub(in crate::process) fn insert_process_group(&mut self, process_group: &Arc<ProcessGroup>) {
        let old_process_group = self
            .inner
            .process_groups
            .insert(process_group.pgid(), Arc::downgrade(process_group));
        debug_assert!(old_process_group.is_none());
    }

    pub(in crate::process) fn remove_process_group(&mut self, pgid: &Pgid) {
        self.inner.process_groups.remove(pgid);
    }

    pub(in crate::process) fn is_empty(&self) -> bool {
        self.inner.process_groups.is_empty()
    }
}
