use core::{
    cell::UnsafeCell,
    fmt,
    ops::{Deref, DerefMut},
    sync::atomic::{
        AtomicUsize,
        Ordering::{AcqRel, Acquire, Relaxed, Release},
    },
};

use super::WaitQueue;

pub struct RwMutex<T: ?Sized> {
    lock: AtomicUsize,
    queue: WaitQueue,
    val: UnsafeCell<T>,
}

const READER: usize = 1;
const WRITER: usize = 1 << (usize::BITS - 1);
const UPGRADEABLE_READER: usize = 1 << (usize::BITS - 2);
const BEING_UPGRADED: usize = 1 << (usize::BITS - 3);

const MAX_READER: usize = 1 << (usize::BITS - 4);

impl<T> RwMutex<T> {
    pub const fn new(val: T) -> Self {
        Self {
            lock: AtomicUsize::new(0),
            queue: WaitQueue::new(),
            val: UnsafeCell::new(val),
        }
    }
}

impl<T: ?Sized> RwMutex<T> {
    #[track_caller]
    pub fn read(&self) -> RwMutexReadGuard<'_, T> {
        self.queue.wait_until(|| self.try_read())
    }

    #[track_caller]
    pub fn write(&self) -> RwMutexWriteGuard<'_, T> {
        self.queue.wait_until(|| self.try_write())
    }

    #[track_caller]
    pub fn upread(&self) -> RwMutexUpgradeableGuard<'_, T> {
        self.queue.wait_until(|| self.try_upread())
    }

    pub fn try_read(&self) -> Option<RwMutexReadGuard<'_, T>> {
        let lock = self.lock.fetch_add(READER, Acquire);
        if lock & (WRITER | BEING_UPGRADED | MAX_READER) == 0 {
            Some(RwMutexReadGuard { inner: self })
        } else {
            self.lock.fetch_sub(READER, Release);
            None
        }
    }

    pub fn try_write(&self) -> Option<RwMutexWriteGuard<'_, T>> {
        if self
            .lock
            .compare_exchange(0, WRITER, Acquire, Relaxed)
            .is_ok()
        {
            Some(RwMutexWriteGuard { inner: self })
        } else {
            None
        }
    }

    pub fn try_upread(&self) -> Option<RwMutexUpgradeableGuard<'_, T>> {
        let lock = self.lock.fetch_or(UPGRADEABLE_READER, Acquire) & (WRITER | UPGRADEABLE_READER);
        if lock == 0 {
            return Some(RwMutexUpgradeableGuard { inner: self });
        } else if lock == WRITER {
            self.lock.fetch_sub(UPGRADEABLE_READER, Release);
        }
        None
    }

    pub fn get_mut(&mut self) -> &mut T {
        self.val.get_mut()
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for RwMutex<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&self.val, f)
    }
}

unsafe impl<T: ?Sized + Send> Send for RwMutex<T> {}
unsafe impl<T: ?Sized + Send + Sync> Sync for RwMutex<T> {}

impl<T: ?Sized> !Send for RwMutexWriteGuard<'_, T> {}
unsafe impl<T: ?Sized + Sync> Sync for RwMutexWriteGuard<'_, T> {}

impl<T: ?Sized> !Send for RwMutexReadGuard<'_, T> {}
unsafe impl<T: ?Sized + Sync> Sync for RwMutexReadGuard<'_, T> {}

impl<T: ?Sized> !Send for RwMutexUpgradeableGuard<'_, T> {}
unsafe impl<T: ?Sized + Sync> Sync for RwMutexUpgradeableGuard<'_, T> {}

pub struct RwMutexReadGuard<'a, T: ?Sized> {
    inner: &'a RwMutex<T>,
}

impl<T: ?Sized> Deref for RwMutexReadGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.inner.val.get() }
    }
}

impl<T: ?Sized> Drop for RwMutexReadGuard<'_, T> {
    fn drop(&mut self) {
        if self.inner.lock.fetch_sub(READER, Release) == READER {
            self.inner.queue.wake_one();
        }
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for RwMutexReadGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

#[clippy::has_significant_drop]
#[must_use]
pub struct RwMutexWriteGuard<'a, T: ?Sized> {
    inner: &'a RwMutex<T>,
}

impl<'a, T: ?Sized> RwMutexWriteGuard<'a, T> {
    pub fn downgrade(mut self) -> RwMutexUpgradeableGuard<'a, T> {
        loop {
            self = match self.try_downgrade() {
                Ok(guard) => return guard,
                Err(e) => e,
            };
        }
    }

    fn try_downgrade(self) -> Result<RwMutexUpgradeableGuard<'a, T>, Self> {
        let inner = self.inner;
        let res = self
            .inner
            .lock
            .compare_exchange(WRITER, UPGRADEABLE_READER, AcqRel, Relaxed);
        if res.is_ok() {
            drop(self);
            Ok(RwMutexUpgradeableGuard { inner })
        } else {
            Err(self)
        }
    }
}

impl<T: ?Sized> Deref for RwMutexWriteGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.inner.val.get() }
    }
}

impl<T: ?Sized> DerefMut for RwMutexWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.inner.val.get() }
    }
}

impl<T: ?Sized> Drop for RwMutexWriteGuard<'_, T> {
    fn drop(&mut self) {
        self.inner.lock.fetch_and(!WRITER, Release);

        self.inner.queue.wake_all();
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for RwMutexWriteGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

pub struct RwMutexUpgradeableGuard<'a, T: ?Sized> {
    inner: &'a RwMutex<T>,
}

impl<'a, T: ?Sized> RwMutexUpgradeableGuard<'a, T> {
    pub fn upgrade(mut self) -> RwMutexWriteGuard<'a, T> {
        self.inner.lock.fetch_or(BEING_UPGRADED, Acquire);
        loop {
            self = match self.try_upgrade() {
                Ok(guard) => return guard,
                Err(e) => e,
            };
        }
    }

    fn try_upgrade(self) -> Result<RwMutexWriteGuard<'a, T>, Self> {
        let res = self.inner.lock.compare_exchange(
            UPGRADEABLE_READER | BEING_UPGRADED,
            WRITER | UPGRADEABLE_READER,
            AcqRel,
            Relaxed,
        );
        if res.is_ok() {
            let inner = self.inner;
            drop(self);
            Ok(RwMutexWriteGuard { inner })
        } else {
            Err(self)
        }
    }
}

impl<T: ?Sized> Deref for RwMutexUpgradeableGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.inner.val.get() }
    }
}

impl<T: ?Sized> Drop for RwMutexUpgradeableGuard<'_, T> {
    fn drop(&mut self) {
        let res = self.inner.lock.fetch_sub(UPGRADEABLE_READER, Release);
        if res == UPGRADEABLE_READER {
            self.inner.queue.wake_all();
        }
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for RwMutexUpgradeableGuard<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}
