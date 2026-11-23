use core::{
    cell::UnsafeCell,
    fmt,
    marker::PhantomData,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicBool, Ordering},
};

use super::{LocalIrqDisabled, PreemptDisabled, guard::SpinGuardian};
use crate::task::atomic_mode::AsAtomicModeGuard;

#[repr(transparent)]
pub struct SpinLock<T: ?Sized, G = PreemptDisabled> {
    phantom: PhantomData<G>,
    inner: SpinLockInner<T>,
}

struct SpinLockInner<T: ?Sized> {
    lock: AtomicBool,
    val: UnsafeCell<T>,
}

impl<T, G> SpinLock<T, G> {
    pub const fn new(val: T) -> Self {
        let lock_inner = SpinLockInner {
            lock: AtomicBool::new(false),
            val: UnsafeCell::new(val),
        };
        Self {
            phantom: PhantomData,
            inner: lock_inner,
        }
    }
}

impl<T: ?Sized> SpinLock<T, PreemptDisabled> {
    pub fn disable_irq(&self) -> &SpinLock<T, LocalIrqDisabled> {
        let ptr = self as *const SpinLock<T, PreemptDisabled>;
        let ptr = ptr as *const SpinLock<T, LocalIrqDisabled>;
        unsafe { &*ptr }
    }
}

impl<T: ?Sized, G: SpinGuardian> SpinLock<T, G> {
    pub fn lock(&self) -> SpinLockGuard<'_, T, G> {
        let inner_guard = G::guard();
        self.acquire_lock();
        SpinLockGuard {
            guard: inner_guard,
            lock: self,
        }
    }

    pub fn try_lock(&self) -> Option<SpinLockGuard<'_, T, G>> {
        let inner_guard = G::guard();
        if self.try_acquire_lock() {
            let lock_guard = SpinLockGuard {
                guard: inner_guard,
                lock: self,
            };
            return Some(lock_guard);
        }
        None
    }

    pub fn get_mut(&mut self) -> &mut T {
        self.inner.val.get_mut()
    }

    fn acquire_lock(&self) {
        while !self.try_acquire_lock() {
            core::hint::spin_loop();
        }
    }

    fn try_acquire_lock(&self) -> bool {
        self.inner
            .lock
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }

    unsafe fn release_lock(&self) {
        self.inner.lock.store(false, Ordering::Release);
    }
}

impl<T: ?Sized + fmt::Debug, G> fmt::Debug for SpinLock<T, G> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&self.inner.val, f)
    }
}

unsafe impl<T: ?Sized + Send, G> Send for SpinLock<T, G> {}
unsafe impl<T: ?Sized + Send, G> Sync for SpinLock<T, G> {}

#[clippy::has_significant_drop]
#[must_use]
pub struct SpinLockGuard<'a, T: ?Sized, G: SpinGuardian> {
    guard: G::Guard,
    lock: &'a SpinLock<T, G>,
}

impl<T: ?Sized, G: SpinGuardian> AsAtomicModeGuard for SpinLockGuard<'_, T, G> {
    fn as_atomic_mode_guard(&self) -> &dyn crate::task::atomic_mode::InAtomicMode {
        self.guard.as_atomic_mode_guard()
    }
}

impl<T: ?Sized, G: SpinGuardian> Deref for SpinLockGuard<'_, T, G> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.lock.inner.val.get() }
    }
}

impl<T: ?Sized, G: SpinGuardian> DerefMut for SpinLockGuard<'_, T, G> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.lock.inner.val.get() }
    }
}

impl<T: ?Sized, G: SpinGuardian> Drop for SpinLockGuard<'_, T, G> {
    fn drop(&mut self) {
        unsafe { self.lock.release_lock() };
    }
}

impl<T: ?Sized + fmt::Debug, G: SpinGuardian> fmt::Debug for SpinLockGuard<'_, T, G> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl<T: ?Sized, G: SpinGuardian> !Send for SpinLockGuard<'_, T, G> {}
impl<T: ?Sized, G: SpinGuardian> !Sync for SpinLockGuard<'_, T, G> {}
