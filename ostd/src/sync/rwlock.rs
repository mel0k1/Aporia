use core::{
    cell::UnsafeCell,
    fmt,
    marker::PhantomData,
    ops::{Deref, DerefMut},
    sync::atomic::{
        AtomicUsize,
        Ordering::{AcqRel, Acquire, Relaxed, Release},
    },
};

use super::{
    PreemptDisabled,
    guard::{GuardTransfer, SpinGuardian},
};
use crate::task::atomic_mode::AsAtomicModeGuard;

pub struct RwLock<T: ?Sized, Guard = PreemptDisabled> {
    guard: PhantomData<Guard>,
    lock: AtomicUsize,
    val: UnsafeCell<T>,
}

const READER: usize = 1;
const WRITER: usize = 1 << (usize::BITS - 1);
const UPGRADEABLE_READER: usize = 1 << (usize::BITS - 2);
const BEING_UPGRADED: usize = 1 << (usize::BITS - 3);

const MAX_READER: usize = 1 << (usize::BITS - 4);

impl<T, G> RwLock<T, G> {
    pub const fn new(val: T) -> Self {
        Self {
            guard: PhantomData,
            lock: AtomicUsize::new(0),
            val: UnsafeCell::new(val),
        }
    }
}

impl<T: ?Sized, G: SpinGuardian> RwLock<T, G> {
    pub fn read(&self) -> RwLockReadGuard<'_, T, G> {
        loop {
            if let Some(readguard) = self.try_read() {
                return readguard;
            } else {
                core::hint::spin_loop();
            }
        }
    }

    pub fn write(&self) -> RwLockWriteGuard<'_, T, G> {
        loop {
            if let Some(writeguard) = self.try_write() {
                return writeguard;
            } else {
                core::hint::spin_loop();
            }
        }
    }

    pub fn upread(&self) -> RwLockUpgradeableGuard<'_, T, G> {
        loop {
            if let Some(guard) = self.try_upread() {
                return guard;
            } else {
                core::hint::spin_loop();
            }
        }
    }

    pub fn try_read(&self) -> Option<RwLockReadGuard<'_, T, G>> {
        let guard = G::read_guard();
        let lock = self.lock.fetch_add(READER, Acquire);
        if lock & (WRITER | MAX_READER | BEING_UPGRADED) == 0 {
            Some(RwLockReadGuard { guard, inner: self })
        } else {
            self.lock.fetch_sub(READER, Release);
            None
        }
    }

    pub fn try_write(&self) -> Option<RwLockWriteGuard<'_, T, G>> {
        let guard = G::guard();
        if self
            .lock
            .compare_exchange(0, WRITER, Acquire, Relaxed)
            .is_ok()
        {
            Some(RwLockWriteGuard { guard, inner: self })
        } else {
            None
        }
    }

    pub fn try_upread(&self) -> Option<RwLockUpgradeableGuard<'_, T, G>> {
        let guard = G::guard();
        let lock = self.lock.fetch_or(UPGRADEABLE_READER, Acquire) & (WRITER | UPGRADEABLE_READER);
        if lock == 0 {
            return Some(RwLockUpgradeableGuard { guard, inner: self });
        } else if lock == WRITER {
            self.lock.fetch_sub(UPGRADEABLE_READER, Release);
        }
        None
    }

    pub fn get_mut(&mut self) -> &mut T {
        self.val.get_mut()
    }

    pub(super) fn as_ptr(&self) -> *mut T {
        self.val.get()
    }
}

impl<T: ?Sized + fmt::Debug, G> fmt::Debug for RwLock<T, G> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&self.val, f)
    }
}

unsafe impl<T: ?Sized + Send, G> Send for RwLock<T, G> {}
unsafe impl<T: ?Sized + Send + Sync, G> Sync for RwLock<T, G> {}

impl<T: ?Sized, G: SpinGuardian> !Send for RwLockWriteGuard<'_, T, G> {}
impl<T: ?Sized, G: SpinGuardian> !Sync for RwLockWriteGuard<'_, T, G> {}

impl<T: ?Sized, G: SpinGuardian> !Send for RwLockReadGuard<'_, T, G> {}
impl<T: ?Sized, G: SpinGuardian> !Sync for RwLockReadGuard<'_, T, G> {}

impl<T: ?Sized, G: SpinGuardian> !Send for RwLockUpgradeableGuard<'_, T, G> {}
impl<T: ?Sized, G: SpinGuardian> !Sync for RwLockUpgradeableGuard<'_, T, G> {}

#[clippy::has_significant_drop]
#[must_use]
pub struct RwLockReadGuard<'a, T: ?Sized, G: SpinGuardian> {
    guard: G::ReadGuard,
    inner: &'a RwLock<T, G>,
}

impl<T: ?Sized, G: SpinGuardian> AsAtomicModeGuard for RwLockReadGuard<'_, T, G> {
    fn as_atomic_mode_guard(&self) -> &dyn crate::task::atomic_mode::InAtomicMode {
        self.guard.as_atomic_mode_guard()
    }
}

impl<T: ?Sized, G: SpinGuardian> Deref for RwLockReadGuard<'_, T, G> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.inner.val.get() }
    }
}

impl<T: ?Sized, G: SpinGuardian> Drop for RwLockReadGuard<'_, T, G> {
    fn drop(&mut self) {
        self.inner.lock.fetch_sub(READER, Release);
    }
}

impl<T: ?Sized + fmt::Debug, G: SpinGuardian> fmt::Debug for RwLockReadGuard<'_, T, G> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

pub struct RwLockWriteGuard<'a, T: ?Sized, G: SpinGuardian> {
    guard: G::Guard,
    inner: &'a RwLock<T, G>,
}

impl<T: ?Sized, G: SpinGuardian> AsAtomicModeGuard for RwLockWriteGuard<'_, T, G> {
    fn as_atomic_mode_guard(&self) -> &dyn crate::task::atomic_mode::InAtomicMode {
        self.guard.as_atomic_mode_guard()
    }
}

impl<T: ?Sized, G: SpinGuardian> Deref for RwLockWriteGuard<'_, T, G> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.inner.val.get() }
    }
}

impl<T: ?Sized, G: SpinGuardian> DerefMut for RwLockWriteGuard<'_, T, G> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.inner.val.get() }
    }
}

impl<T: ?Sized, G: SpinGuardian> Drop for RwLockWriteGuard<'_, T, G> {
    fn drop(&mut self) {
        self.inner.lock.fetch_and(!WRITER, Release);
    }
}

impl<T: ?Sized + fmt::Debug, G: SpinGuardian> fmt::Debug for RwLockWriteGuard<'_, T, G> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

pub struct RwLockUpgradeableGuard<'a, T: ?Sized, G: SpinGuardian> {
    guard: G::Guard,
    inner: &'a RwLock<T, G>,
}

impl<T: ?Sized, G: SpinGuardian> AsAtomicModeGuard for RwLockUpgradeableGuard<'_, T, G> {
    fn as_atomic_mode_guard(&self) -> &dyn crate::task::atomic_mode::InAtomicMode {
        self.guard.as_atomic_mode_guard()
    }
}

impl<'a, T: ?Sized, G: SpinGuardian> RwLockUpgradeableGuard<'a, T, G> {
    pub fn upgrade(mut self) -> RwLockWriteGuard<'a, T, G> {
        self.inner.lock.fetch_or(BEING_UPGRADED, Acquire);
        loop {
            self = match self.try_upgrade() {
                Ok(guard) => return guard,
                Err(e) => e,
            };
        }
    }

    fn try_upgrade(mut self) -> Result<RwLockWriteGuard<'a, T, G>, Self> {
        let res = self.inner.lock.compare_exchange(
            UPGRADEABLE_READER | BEING_UPGRADED,
            WRITER | UPGRADEABLE_READER,
            AcqRel,
            Relaxed,
        );
        if res.is_ok() {
            let guard = self.guard.transfer_to();
            let inner = self.inner;
            drop(self);
            Ok(RwLockWriteGuard { guard, inner })
        } else {
            Err(self)
        }
    }
}

impl<T: ?Sized, G: SpinGuardian> Deref for RwLockUpgradeableGuard<'_, T, G> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.inner.val.get() }
    }
}

impl<T: ?Sized, G: SpinGuardian> Drop for RwLockUpgradeableGuard<'_, T, G> {
    fn drop(&mut self) {
        self.inner.lock.fetch_sub(UPGRADEABLE_READER, Release);
    }
}

impl<T: ?Sized + fmt::Debug, G: SpinGuardian> fmt::Debug for RwLockUpgradeableGuard<'_, T, G> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}
