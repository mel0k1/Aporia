use core::{
    marker::PhantomData,
    mem::ManuallyDrop,
    ops::Deref,
    ptr::NonNull,
    sync::atomic::{
        AtomicPtr,
        Ordering::{AcqRel, Acquire},
    },
};

use non_null::NonNullPtr;
use spin::once::Once;

use self::monitor::RcuMonitor;
use crate::{
    panic::PanicGuard,
    task::{
        DisabledPreemptGuard,
        atomic_mode::{AsAtomicModeGuard, InAtomicMode},
        disable_preempt,
    },
};

mod monitor;
pub mod non_null;

pub struct Rcu<P: NonNullPtr>(RcuInner<P>);

#[clippy::has_significant_drop]
#[must_use]
pub struct RcuReadGuard<'a, P: NonNullPtr>(RcuReadGuardInner<'a, P>);

pub struct RcuOption<P: NonNullPtr>(RcuInner<P>);

#[clippy::has_significant_drop]
#[must_use]
pub struct RcuOptionReadGuard<'a, P: NonNullPtr>(RcuReadGuardInner<'a, P>);

struct RcuInner<P: NonNullPtr> {
    ptr: AtomicPtr<<P as NonNullPtr>::Target>,
    _marker: PhantomData<*const P::Target>,
}

unsafe impl<P: NonNullPtr> Send for RcuInner<P> where P: Send {}

unsafe impl<P: NonNullPtr> Sync for RcuInner<P> where P: Send + Sync {}

impl<P: NonNullPtr + Send> RcuInner<P> {
    const fn new_none() -> Self {
        Self {
            ptr: AtomicPtr::new(core::ptr::null_mut()),
            _marker: PhantomData,
        }
    }

    fn new(pointer: P) -> Self {
        let ptr = <P as NonNullPtr>::into_raw(pointer).as_ptr();
        let ptr = AtomicPtr::new(ptr);
        Self {
            ptr,
            _marker: PhantomData,
        }
    }

    fn update(&self, new_ptr: Option<P>) {
        let new_ptr = if let Some(new_ptr) = new_ptr {
            <P as NonNullPtr>::into_raw(new_ptr).as_ptr()
        } else {
            core::ptr::null_mut()
        };

        let old_raw_ptr = self.ptr.swap(new_ptr, AcqRel);

        if let Some(p) = NonNull::new(old_raw_ptr) {
            unsafe { delay_drop::<P>(p) };
        }
    }

    fn read(&self) -> RcuReadGuardInner<'_, P> {
        let guard = disable_preempt();
        RcuReadGuardInner {
            obj_ptr: self.ptr.load(Acquire),
            rcu: self,
            inner_guard: guard,
        }
    }

    fn read_with<'a>(&'a self, _guard: &'a dyn InAtomicMode) -> Option<P::Ref<'a>> {
        let obj_ptr = self.ptr.load(Acquire);
        if obj_ptr.is_null() {
            return None;
        }
        NonNull::new(obj_ptr).map(|ptr| unsafe { P::raw_as_ref(ptr) })
    }
}

impl<P: NonNullPtr> Drop for RcuInner<P> {
    fn drop(&mut self) {
        let ptr = self.ptr.load(Acquire);
        if let Some(p) = NonNull::new(ptr) {
            let pointer = unsafe { <P as NonNullPtr>::from_raw(p) };
            drop(pointer);
        }
    }
}

struct RcuReadGuardInner<'a, P: NonNullPtr> {
    obj_ptr: *mut <P as NonNullPtr>::Target,
    rcu: &'a RcuInner<P>,
    inner_guard: DisabledPreemptGuard,
}

impl<P: NonNullPtr + Send> RcuReadGuardInner<'_, P> {
    fn get(&self) -> Option<P::Ref<'_>> {
        NonNull::new(self.obj_ptr).map(|ptr| unsafe { P::raw_as_ref(ptr) })
    }

    fn compare_exchange(self, new_ptr: Option<P>) -> Result<(), Option<P>> {
        let new_ptr = if let Some(new_ptr) = new_ptr {
            <P as NonNullPtr>::into_raw(new_ptr).as_ptr()
        } else {
            core::ptr::null_mut()
        };

        if self
            .rcu
            .ptr
            .compare_exchange(self.obj_ptr, new_ptr, AcqRel, Acquire)
            .is_err()
        {
            let Some(new_ptr) = NonNull::new(new_ptr) else {
                return Err(None);
            };
            return Err(Some(unsafe { <P as NonNullPtr>::from_raw(new_ptr) }));
        }

        if let Some(p) = NonNull::new(self.obj_ptr) {
            unsafe { delay_drop::<P>(p) };
        }

        Ok(())
    }
}

impl<P: NonNullPtr + Send> Rcu<P> {
    pub fn new(pointer: P) -> Self {
        Self(RcuInner::new(pointer))
    }

    pub fn update(&self, new_ptr: P) {
        self.0.update(Some(new_ptr));
    }

    pub fn read(&self) -> RcuReadGuard<'_, P> {
        RcuReadGuard(self.0.read())
    }

    pub fn read_with<'a, G: AsAtomicModeGuard + ?Sized>(&'a self, guard: &'a G) -> P::Ref<'a> {
        self.0.read_with(guard.as_atomic_mode_guard()).unwrap()
    }
}

impl<P: NonNullPtr + Send> RcuOption<P> {
    pub fn new(pointer: Option<P>) -> Self {
        if let Some(pointer) = pointer {
            Self(RcuInner::new(pointer))
        } else {
            Self(RcuInner::new_none())
        }
    }

    pub const fn new_none() -> Self {
        Self(RcuInner::new_none())
    }

    pub fn update(&self, new_ptr: Option<P>) {
        self.0.update(new_ptr);
    }

    pub fn read(&self) -> RcuOptionReadGuard<'_, P> {
        RcuOptionReadGuard(self.0.read())
    }

    pub fn read_with<'a, G: AsAtomicModeGuard + ?Sized>(
        &'a self,
        guard: &'a G,
    ) -> Option<P::Ref<'a>> {
        self.0.read_with(guard.as_atomic_mode_guard())
    }
}

impl<P: NonNullPtr + Send> RcuReadGuard<'_, P> {
    pub fn get(&self) -> P::Ref<'_> {
        self.0.get().unwrap()
    }

    pub fn compare_exchange(self, new_ptr: P) -> Result<(), P> {
        self.0
            .compare_exchange(Some(new_ptr))
            .map_err(|err| err.unwrap())
    }
}

impl<P: NonNullPtr> AsAtomicModeGuard for RcuReadGuard<'_, P> {
    fn as_atomic_mode_guard(&self) -> &dyn InAtomicMode {
        self.0.inner_guard.as_atomic_mode_guard()
    }
}

impl<P: NonNullPtr + Send> RcuOptionReadGuard<'_, P> {
    pub fn get(&self) -> Option<P::Ref<'_>> {
        self.0.get()
    }

    pub fn is_none(&self) -> bool {
        self.0.obj_ptr.is_null()
    }

    pub fn compare_exchange(self, new_ptr: Option<P>) -> Result<(), Option<P>> {
        self.0.compare_exchange(new_ptr)
    }
}

impl<P: NonNullPtr> AsAtomicModeGuard for RcuOptionReadGuard<'_, P> {
    fn as_atomic_mode_guard(&self) -> &dyn InAtomicMode {
        self.0.inner_guard.as_atomic_mode_guard()
    }
}

unsafe fn delay_drop<P: NonNullPtr + Send>(pointer: NonNull<<P as NonNullPtr>::Target>) {
    struct ForceSend<P: NonNullPtr + Send>(NonNull<<P as NonNullPtr>::Target>);
    unsafe impl<P: NonNullPtr + Send> Send for ForceSend<P> {}

    let pointer: ForceSend<P> = ForceSend(pointer);

    let rcu_monitor = RCU_MONITOR.get().unwrap();
    rcu_monitor.after_grace_period(move || {
        let pointer = pointer;

        let p = unsafe { <P as NonNullPtr>::from_raw(pointer.0) };
        drop(p);
    });
}

#[repr(transparent)]
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RcuDrop<T: Send + 'static> {
    value: ManuallyDrop<T>,
}

impl<T: Send + 'static> RcuDrop<T> {
    pub fn new(value: T) -> Self {
        Self {
            value: ManuallyDrop::new(value),
        }
    }

    pub(crate) unsafe fn into_inner(slot: RcuDrop<T>) -> (T, PanicGuard) {
        let mut slot = ManuallyDrop::new(slot);
        let panic_guard = PanicGuard::new();
        let val = unsafe { ManuallyDrop::take(&mut slot.value) };
        (val, panic_guard)
    }
}

impl<T: Send + 'static> Deref for RcuDrop<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T: Send + 'static> Drop for RcuDrop<T> {
    fn drop(&mut self) {
        let taken = unsafe { ManuallyDrop::take(&mut self.value) };
        let rcu_monitor = RCU_MONITOR.get().unwrap();
        rcu_monitor.after_grace_period(|| {
            drop(taken);
        });
    }
}

pub(crate) unsafe fn finish_grace_period() {
    let rcu_monitor = RCU_MONITOR.get().unwrap();
    unsafe {
        rcu_monitor.finish_grace_period();
    }
}

static RCU_MONITOR: Once<RcuMonitor> = Once::new();

pub(super) fn init() {
    RCU_MONITOR.call_once(RcuMonitor::new);
}
