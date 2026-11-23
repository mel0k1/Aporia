use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering, fence};

use super::{PreemptDisabled, RwLock, RwLockReadGuard, RwLockWriteGuard};

pub struct RwArc<T>(Arc<Inner<T>>);

pub struct RoArc<T>(Arc<Inner<T>>);

struct Inner<T> {
    data: RwLock<T>,
    num_rw: AtomicUsize,
}

impl<T> RwArc<T> {
    pub fn new(data: T) -> Self {
        let inner = Inner {
            data: RwLock::new(data),
            num_rw: AtomicUsize::new(1),
        };
        Self(Arc::new(inner))
    }

    pub fn read(&self) -> RwLockReadGuard<'_, T, PreemptDisabled> {
        self.0.data.read()
    }

    pub fn write(&self) -> RwLockWriteGuard<'_, T, PreemptDisabled> {
        self.0.data.write()
    }

    pub fn as_ptr(&self) -> *const T {
        self.0.data.as_ptr()
    }

    pub fn get(&mut self) -> Option<&T> {
        if self.0.num_rw.load(Ordering::Relaxed) > 1 {
            return None;
        }

        fence(Ordering::Acquire);

        let data_ptr = self.0.data.as_ptr();

        Some(unsafe { &*data_ptr })
    }

    pub fn clone_ro(&self) -> RoArc<T> {
        RoArc(self.0.clone())
    }
}

impl<T> Clone for RwArc<T> {
    fn clone(&self) -> Self {
        let inner = self.0.clone();

        inner.num_rw.fetch_add(1, Ordering::Relaxed);

        Self(inner)
    }
}

impl<T> Drop for RwArc<T> {
    fn drop(&mut self) {
        self.0.num_rw.fetch_sub(1, Ordering::Release);
    }
}

impl<T: Clone> RwArc<T> {
    pub fn get_cloned(&self) -> T {
        let guard = self.read();
        guard.clone()
    }
}

impl<T> RoArc<T> {
    pub fn read(&self) -> RwLockReadGuard<'_, T, PreemptDisabled> {
        self.0.data.read()
    }
}

#[cfg(ktest)]
mod test {
    use super::*;
    use crate::prelude::*;

    #[ktest]
    fn lockless_get() {
        let mut rw1 = RwArc::new(1u32);
        assert_eq!(rw1.get(), Some(1).as_ref());

        let _ro = rw1.clone_ro();
        assert_eq!(rw1.get(), Some(1).as_ref());

        let rw2 = rw1.clone();
        assert_eq!(rw1.get(), None);

        drop(rw2);
        assert_eq!(rw1.get(), Some(1).as_ref());
    }
}
