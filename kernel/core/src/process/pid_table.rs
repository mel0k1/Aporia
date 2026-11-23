use alloc::collections::btree_map::Entry;

use super::{Pgid, Pid, Process, ProcessGroup, Session, Sid};
use crate::{
    prelude::*,
    process::posix_thread::AsPosixThread,
    thread::{Thread, Tid},
};

static PID_TABLE: Mutex<PidTable> = Mutex::new(PidTable::new());

pub(crate) struct PidTable {
    entries: BTreeMap<u32, Arc<PidEntry>>,
    process_count: usize,
}

impl PidTable {
    const fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            process_count: 0,
        }
    }

    fn get_or_create_entry(&mut self, id: u32) -> &Arc<PidEntry> {
        self.entries
            .entry(id)
            .or_insert_with(|| Arc::new(PidEntry::new()))
    }

    pub(super) fn insert_thread(&mut self, tid: Tid, thread: &Arc<Thread>) {
        debug_assert_eq!(tid, thread.as_posix_thread().unwrap().tid());

        let mut entry = self.get_or_create_entry(tid).lock();
        debug_assert!(!entry.has_live_process());

        entry.set_thread(thread);
    }

    pub(super) fn remove_thread(&mut self, tid: Tid) {
        let Entry::Occupied(map_entry) = self.entries.entry(tid) else {
            return;
        };

        let should_remove = {
            let mut pid_entry = map_entry.get().lock();
            debug_assert!(!pid_entry.has_live_process());

            pid_entry.clear_thread();
            pid_entry.is_empty()
        };

        if should_remove {
            map_entry.remove();
        }
    }

    pub(super) fn take_thread(&mut self, tid: Tid) -> Option<Arc<Thread>> {
        let Entry::Occupied(map_entry) = self.entries.entry(tid) else {
            return None;
        };

        let (thread, should_remove) = {
            let mut pid_entry = map_entry.get().lock();
            debug_assert!(!pid_entry.has_live_process());

            let thread = pid_entry.thread()?;
            pid_entry.clear_thread();
            (thread, pid_entry.is_empty())
        };

        if should_remove {
            map_entry.remove();
        }

        Some(thread)
    }

    pub(super) fn replace_thread(&mut self, tid: Tid, thread: &Arc<Thread>) {
        debug_assert_eq!(tid, thread.as_posix_thread().unwrap().tid());

        let entry = self.get_or_create_entry(tid);
        entry.lock().replace_thread(thread);
    }

    pub(crate) fn get_thread(&self, tid: Tid) -> Option<Arc<Thread>> {
        self.entries
            .get(&tid)
            .and_then(|entry| entry.lock().thread())
    }

    pub(crate) fn iter_threads(&self) -> impl Iterator<Item = Arc<Thread>> + '_ {
        self.entries
            .values()
            .filter_map(|entry| entry.lock().thread())
    }

    pub(super) fn insert_process(&mut self, pid: Pid, process: &Arc<Process>) {
        self.process_count += 1;

        let entry = self.get_or_create_entry(pid);
        let mut entry = entry.lock();
        entry.set_process(process);
        entry.set_thread(&process.main_thread());
    }

    pub(super) fn remove_process(&mut self, pid: Pid) {
        let Entry::Occupied(map_entry) = self.entries.entry(pid) else {
            return;
        };

        let should_remove = {
            let mut pid_entry = map_entry.get().lock();

            self.process_count -= 1;

            pid_entry.clear_process();
            pid_entry.clear_thread();
            pid_entry.is_empty()
        };

        if should_remove {
            map_entry.remove();
        }
    }

    pub(crate) fn get_process(&self, pid: Pid) -> Option<Arc<Process>> {
        self.entries
            .get(&pid)
            .and_then(|entry| entry.lock().process())
    }

    pub(crate) fn iter_processes(&self) -> impl Iterator<Item = Arc<Process>> + '_ {
        self.entries
            .values()
            .filter_map(|entry| entry.lock().process())
    }

    pub(crate) fn process_count(&self) -> usize {
        self.process_count
    }

    pub(super) fn insert_process_group(&mut self, pgid: Pgid, group: &Arc<ProcessGroup>) {
        let entry = self.get_or_create_entry(pgid);
        entry.lock().set_process_group(group);
    }

    pub(super) fn remove_process_group(&mut self, pgid: Pgid) {
        let Entry::Occupied(map_entry) = self.entries.entry(pgid) else {
            return;
        };

        let should_remove = {
            let mut pid_entry = map_entry.get().lock();
            pid_entry.clear_process_group();
            pid_entry.is_empty()
        };

        if should_remove {
            map_entry.remove();
        }
    }

    pub(crate) fn get_process_group(&self, pgid: &Pgid) -> Option<Arc<ProcessGroup>> {
        self.entries
            .get(pgid)
            .and_then(|entry| entry.lock().process_group())
    }

    pub(crate) fn contains_process_group(&self, pgid: &Pgid) -> bool {
        self.entries
            .get(pgid)
            .is_some_and(|entry| entry.lock().has_live_process_group())
    }

    pub(super) fn insert_session(&mut self, sid: Sid, session: &Arc<Session>) {
        let entry = self.get_or_create_entry(sid);
        entry.lock().set_session(session);
    }

    pub(super) fn remove_session(&mut self, sid: Sid) {
        let Entry::Occupied(map_entry) = self.entries.entry(sid) else {
            return;
        };

        let should_remove = {
            let mut pid_entry = map_entry.get().lock();
            pid_entry.clear_session();
            pid_entry.is_empty()
        };

        if should_remove {
            map_entry.remove();
        }
    }

    pub(crate) fn get_entry(&self, id: u32) -> Option<Arc<PidEntry>> {
        self.entries.get(&id).cloned()
    }
}

pub(crate) struct PidEntry {
    inner: Mutex<PidEntryInner>,
}

struct PidEntryInner {
    thread: Weak<Thread>,
    process: Weak<Process>,
    process_group: Weak<ProcessGroup>,
    session: Weak<Session>,
}

pub(crate) enum PidEntryType {
    Process,
    Thread,
}

impl PidEntry {
    fn new() -> Self {
        Self {
            inner: Mutex::new(PidEntryInner::new()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, PidEntryInner> {
        self.inner.lock()
    }

    pub(crate) fn thread(&self) -> Option<Arc<Thread>> {
        self.lock().thread()
    }

    pub(crate) fn process_of_thread(&self) -> Option<Arc<Process>> {
        let inner = self.lock();

        if let Some(process) = inner.process() {
            return Some(process);
        }

        if let Some(thread) = inner.thread() {
            return Some(thread.as_posix_thread().unwrap().process());
        }

        None
    }

    pub(crate) fn type_(&self) -> Option<PidEntryType> {
        let inner = self.lock();

        if inner.has_live_process() {
            return Some(PidEntryType::Process);
        }

        if inner.has_live_thread() {
            return Some(PidEntryType::Thread);
        }

        None
    }
}

impl PidEntryInner {
    fn new() -> Self {
        Self {
            thread: Weak::new(),
            process: Weak::new(),
            process_group: Weak::new(),
            session: Weak::new(),
        }
    }

    fn set_thread(&mut self, thread: &Arc<Thread>) {
        debug_assert!(!self.has_live_thread());
        self.thread = Arc::downgrade(thread);
    }

    fn clear_thread(&mut self) {
        debug_assert!(self.has_live_thread());
        self.thread = Weak::new();
    }

    fn replace_thread(&mut self, thread: &Arc<Thread>) {
        debug_assert!(self.has_live_thread());
        self.thread = Arc::downgrade(thread);
    }

    fn set_process(&mut self, process: &Arc<Process>) {
        debug_assert!(!self.has_live_process());
        self.process = Arc::downgrade(process);
    }

    fn clear_process(&mut self) {
        debug_assert!(self.has_live_process());
        self.process = Weak::new();
    }

    fn set_process_group(&mut self, group: &Arc<ProcessGroup>) {
        debug_assert!(!self.has_live_process_group());
        self.process_group = Arc::downgrade(group);
    }

    fn clear_process_group(&mut self) {
        debug_assert!(self.has_live_process_group());
        self.process_group = Weak::new();
    }

    fn set_session(&mut self, session: &Arc<Session>) {
        debug_assert!(!self.has_live_session());
        self.session = Arc::downgrade(session);
    }

    fn clear_session(&mut self) {
        debug_assert!(self.has_live_session());
        self.session = Weak::new();
    }

    fn thread(&self) -> Option<Arc<Thread>> {
        self.thread.upgrade()
    }

    fn process(&self) -> Option<Arc<Process>> {
        self.process.upgrade()
    }

    fn process_group(&self) -> Option<Arc<ProcessGroup>> {
        self.process_group.upgrade()
    }

    fn has_live_thread(&self) -> bool {
        !self.thread.is_dangling()
    }

    fn has_live_process(&self) -> bool {
        !self.process.is_dangling()
    }

    fn has_live_process_group(&self) -> bool {
        !self.process_group.is_dangling()
    }

    fn has_live_session(&self) -> bool {
        !self.session.is_dangling()
    }

    fn is_empty(&self) -> bool {
        !self.has_live_thread()
            && !self.has_live_process()
            && !self.has_live_process_group()
            && !self.has_live_session()
    }
}

pub(crate) fn pid_table_mut() -> MutexGuard<'static, PidTable> {
    PID_TABLE.lock()
}

trait WeakIsDangling {
    fn is_dangling(&self) -> bool;
}

impl<T> WeakIsDangling for Weak<T> {
    fn is_dangling(&self) -> bool {
        Weak::ptr_eq(self, &Weak::new())
    }
}
