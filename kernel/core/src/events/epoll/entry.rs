use alloc::{
    collections::vec_deque::VecDeque,
    sync::{Arc, Weak},
};
use core::{
    fmt::Display,
    sync::atomic::{AtomicBool, Ordering},
};

use keyable_arc::{KeyableArc, KeyableWeak};
use ostd::sync::{LocalIrqDisabled, Mutex, MutexGuard, SpinLock, SpinLockGuard};

use super::{EpollEvent, EpollFlags};
use crate::{
    events::{self, IoEvents},
    fs::file::{FileLike, file_table::FileDesc},
    process::signal::{PollHandle, Pollee},
};

pub(super) struct Entry {
    key: EntryKey,
    inner: Mutex<Inner>,
    observer: Arc<Observer>,
}

impl Display for Entry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let inner = self.inner.lock();
        write!(f, "tfd: {:8} ", self.key.fd)?;
        write!(f, "events: {:8x} ", inner.event.events.bits())?;
        write!(f, "data: {:16x}", inner.event.user_data)
    }
}

#[derive(Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct EntryKey {
    fd: FileDesc,
    file: KeyableWeak<dyn FileLike>,
}

impl From<(FileDesc, KeyableWeak<dyn FileLike>)> for EntryKey {
    fn from(value: (FileDesc, KeyableWeak<dyn FileLike>)) -> Self {
        Self {
            fd: value.0,
            file: value.1,
        }
    }
}

impl From<(FileDesc, &Arc<dyn FileLike>)> for EntryKey {
    fn from(value: (FileDesc, &Arc<dyn FileLike>)) -> Self {
        Self {
            fd: value.0,
            file: KeyableWeak::from(Arc::downgrade(value.1)),
        }
    }
}

struct Inner {
    event: EpollEvent,
    flags: EpollFlags,
    poller: PollHandle,
}

impl Entry {
    pub(super) fn new(
        fd: FileDesc,
        file: KeyableWeak<dyn FileLike>,
        ready_set: Arc<ReadySet>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|me| {
            let observer = Arc::new(Observer::new(ready_set, me.clone()));

            let inner = Inner {
                event: EpollEvent {
                    events: IoEvents::empty(),
                    user_data: 0,
                },
                flags: EpollFlags::empty(),
                poller: PollHandle::new(Arc::downgrade(&observer) as _),
            };

            Self {
                key: EntryKey { fd, file },
                inner: Mutex::new(inner),
                observer,
            }
        })
    }

    fn file(&self) -> Option<Arc<dyn FileLike>> {
        self.key.file.upgrade().map(KeyableArc::into)
    }

    pub(super) fn poll(&self) -> Option<(Option<EpollEvent>, bool)> {
        let file = self.file()?;
        let inner = self.inner.lock();

        if !self.observer.is_enabled() {
            return Some((None, false));
        }

        let io_events = file.poll(inner.event.events, None);

        let ep_event = if !io_events.is_empty() {
            Some(EpollEvent::new(io_events, inner.event.user_data))
        } else {
            None
        };

        let is_still_ready = ep_event.is_some()
            && !inner
                .flags
                .intersects(EpollFlags::EDGE_TRIGGER | EpollFlags::ONE_SHOT);

        if ep_event.is_some() && inner.flags.contains(EpollFlags::ONE_SHOT) {
            self.observer.reset_enabled(&inner);
        }

        Some((ep_event, is_still_ready))
    }

    pub(super) fn update(&self, event: EpollEvent, flags: EpollFlags) -> IoEvents {
        let file = self.file().unwrap();

        let mut inner = self.inner.lock();

        inner.event = event;
        inner.flags = flags;

        self.observer.set_enabled(&inner);

        file.poll(event.events, Some(&mut inner.poller))
    }

    pub(super) fn shutdown(&self) {
        let mut inner = self.inner.lock();

        self.observer.reset_enabled(&inner);
        inner.poller.reset();
    }

    pub(super) fn observer(&self) -> &Observer {
        &self.observer
    }

    pub(super) fn key(&self) -> &EntryKey {
        &self.key
    }

    pub(super) fn fd(&self) -> FileDesc {
        self.key.fd
    }

    pub(super) fn file_weak(&self) -> &KeyableWeak<dyn FileLike> {
        &self.key.file
    }
}

pub(super) struct Observer {
    is_enabled: AtomicBool,
    is_ready: AtomicBool,
    ready_set: Arc<ReadySet>,
    weak_entry: Weak<Entry>,
}

impl Observer {
    fn new(ready_set: Arc<ReadySet>, weak_entry: Weak<Entry>) -> Self {
        Self {
            is_enabled: AtomicBool::new(false),
            is_ready: AtomicBool::new(false),
            ready_set,
            weak_entry,
        }
    }

    fn is_ready(&self, _guard: &SpinLockGuard<VecDeque<Weak<Entry>>, LocalIrqDisabled>) -> bool {
        self.is_ready.load(Ordering::Relaxed)
    }

    fn set_ready(&self, _guard: &SpinLockGuard<VecDeque<Weak<Entry>>, LocalIrqDisabled>) {
        self.is_ready.store(true, Ordering::Relaxed);
    }

    fn reset_ready(&self, _guard: &SpinLockGuard<VecDeque<Weak<Entry>>, LocalIrqDisabled>) {
        self.is_ready.store(false, Ordering::Relaxed)
    }

    fn is_enabled(&self) -> bool {
        self.is_enabled.load(Ordering::Relaxed)
    }

    fn set_enabled(&self, _guard: &MutexGuard<Inner>) {
        self.is_enabled.store(true, Ordering::Relaxed)
    }

    fn reset_enabled(&self, _guard: &MutexGuard<Inner>) {
        self.is_enabled.store(false, Ordering::Relaxed)
    }

    fn weak_entry(&self) -> &Weak<Entry> {
        &self.weak_entry
    }
}

impl events::Observer<IoEvents> for Observer {
    fn on_events(&self, _events: &IoEvents) {
        self.ready_set.push(self);
    }
}

pub(super) struct ReadySet {
    entries: SpinLock<VecDeque<Weak<Entry>>, LocalIrqDisabled>,
    pop_guard: Mutex<PopGuard>,
    pollee: Pollee,
}

struct PopGuard;

impl ReadySet {
    pub(super) fn new() -> Self {
        Self {
            entries: SpinLock::new(VecDeque::new()),
            pop_guard: Mutex::new(PopGuard),
            pollee: Pollee::new(),
        }
    }

    pub(super) fn push(&self, observer: &Observer) {
        if !observer.is_enabled() {
            return;
        }

        let mut entries = self.entries.lock();

        if !observer.is_ready(&entries) {
            observer.set_ready(&entries);
            entries.push_back(observer.weak_entry().clone())
        }

        self.pollee.notify(IoEvents::IN);
    }

    pub(super) fn lock_pop(&self) -> ReadySetPopIter<'_> {
        ReadySetPopIter {
            ready_set: self,
            _pop_guard: self.pop_guard.lock(),
            limit: None,
        }
    }

    pub(super) fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee
            .poll_with(mask, poller, || self.check_io_events())
    }

    fn check_io_events(&self) -> IoEvents {
        let entries = self.entries.lock();

        if !entries.is_empty() {
            IoEvents::IN
        } else {
            IoEvents::empty()
        }
    }
}

pub(super) struct ReadySetPopIter<'a> {
    ready_set: &'a ReadySet,
    _pop_guard: MutexGuard<'a, PopGuard>,
    limit: Option<usize>,
}

impl Iterator for ReadySetPopIter<'_> {
    type Item = Arc<Entry>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.limit == Some(0) {
            return None;
        }

        let mut entries = self.ready_set.entries.lock();
        let mut limit = self.limit.unwrap_or_else(|| entries.len());

        while limit > 0 {
            limit -= 1;

            let weak_entry = entries.pop_front().unwrap();

            if entries.is_empty() {
                self.ready_set.pollee.invalidate();
            }

            let Some(entry) = Weak::upgrade(&weak_entry) else {
                continue;
            };

            entry.observer().reset_ready(&entries);

            self.limit = Some(limit);
            return Some(entry);
        }

        self.limit = None;
        None
    }
}
