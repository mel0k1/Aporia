use core::sync::atomic::{AtomicUsize, Ordering};

use keyable_arc::KeyableWeak;
use ostd::sync::LocalIrqDisabled;

use super::{Events, EventsFilter, Observer};
use crate::prelude::*;

pub(crate) struct Subject<E: Events>(BTreeSet<KeyableWeak<dyn Observer<E>>>);

#[expect(dead_code)]
impl<E: Events> Subject<E> {
    pub(crate) const fn new() -> Self {
        Self(BTreeSet::new())
    }

    pub(crate) fn register_observer(&mut self, observer: Weak<dyn Observer<E>>) {
        self.0.insert(KeyableWeak::from(observer));
    }

    pub(crate) fn unregister_observer(&mut self, observer: &Weak<dyn Observer<E>>) -> bool {
        self.0.remove(&KeyableWeak::from(observer.clone()))
    }

    pub(crate) fn notify_observers(&mut self, events: &E) {
        self.0.retain(|observer| {
            if let Some(observer) = observer.upgrade() {
                observer.on_events(events);
                true
            } else {
                false
            }
        });
    }
}

impl<E: Events> Default for Subject<E> {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) struct SyncSubject<E: Events, F: EventsFilter<E> = ()> {
    observers: SpinLock<BTreeMap<KeyableWeak<dyn Observer<E>>, F>, LocalIrqDisabled>,
    num_observers: AtomicUsize,
}

impl<E: Events, F: EventsFilter<E>> SyncSubject<E, F> {
    pub(crate) const fn new() -> Self {
        Self {
            observers: SpinLock::new(BTreeMap::new()),
            num_observers: AtomicUsize::new(0),
        }
    }

    pub(crate) fn register_observer(&self, observer: Weak<dyn Observer<E>>, filter: F) {
        let mut observers = self.observers.lock();
        let is_new = {
            let observer: KeyableWeak<dyn Observer<E>> = observer.into();
            observers.insert(observer, filter).is_none()
        };
        if is_new {
            self.num_observers.fetch_add(1, Ordering::Acquire);
        }
    }

    pub(crate) fn unregister_observer(
        &self,
        observer: &Weak<dyn Observer<E>>,
    ) -> Option<Weak<dyn Observer<E>>> {
        let observer: KeyableWeak<dyn Observer<E>> = observer.clone().into();
        let mut observers = self.observers.lock();
        let observer = observers
            .remove_entry(&observer)
            .map(|(observer, _)| observer.into());
        if observer.is_some() {
            self.num_observers.fetch_sub(1, Ordering::Relaxed);
        }
        observer
    }

    pub(crate) fn notify_observers(&self, events: &E) {
        if self.num_observers.fetch_add(0, Ordering::Release) == 0 {
            return;
        }

        let mut num_freed = 0;
        let mut observers = self.observers.lock();
        observers.retain(|observer, filter| {
            if let Some(observer) = observer.upgrade() {
                if filter.filter(events) {
                    observer.on_events(events);
                }
                true
            } else {
                num_freed += 1;
                false
            }
        });
        if num_freed > 0 {
            self.num_observers.fetch_sub(num_freed, Ordering::Relaxed);
        }
    }
}

impl<E: Events> Default for SyncSubject<E> {
    fn default() -> Self {
        Self::new()
    }
}
