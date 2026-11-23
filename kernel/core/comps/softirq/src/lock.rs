use core::marker::PhantomData;

use ostd::{
    cpu_local_cell,
    irq::{InterruptLevel, disable_local},
    sync::{GuardTransfer, SpinGuardian},
    task::{
        DisabledPreemptGuard,
        atomic_mode::{AsAtomicModeGuard, InAtomicMode},
        disable_preempt,
    },
};

use crate::process_all_pending;

cpu_local_cell! {
    static DISABLE_SOFTIRQ_COUNT: u8 = 0;
}

pub(super) fn is_softirq_enabled() -> bool {
    DISABLE_SOFTIRQ_COUNT.load() == 0
}

pub struct DisableLocalBottomHalfGuard {
    preempt: DisabledPreemptGuard,
}

impl AsAtomicModeGuard for DisableLocalBottomHalfGuard {
    fn as_atomic_mode_guard(&self) -> &dyn InAtomicMode {
        &self.preempt
    }
}

impl Drop for DisableLocalBottomHalfGuard {
    fn drop(&mut self) {
        let in_task_context = InterruptLevel::current().is_task_context();
        if DISABLE_SOFTIRQ_COUNT.load() == 1 && in_task_context {
            let irq_guard = disable_local();
            let irq_guard = process_all_pending(irq_guard);

            DISABLE_SOFTIRQ_COUNT.sub_assign(1);
            drop(irq_guard);
            return;
        }

        DISABLE_SOFTIRQ_COUNT.sub_assign(1);
    }
}

#[must_use]
fn disable_local_bottom_half() -> DisableLocalBottomHalfGuard {
    let preempt = disable_preempt();
    DISABLE_SOFTIRQ_COUNT.add_assign(1);
    DisableLocalBottomHalfGuard { preempt }
}

impl GuardTransfer for DisableLocalBottomHalfGuard {
    fn transfer_to(&mut self) -> Self {
        disable_local_bottom_half()
    }
}

pub enum BottomHalfDisabled {}

impl SpinGuardian for BottomHalfDisabled {
    type Guard = DisableLocalBottomHalfGuard;
    type ReadGuard = DisableLocalBottomHalfGuard;

    fn read_guard() -> Self::ReadGuard {
        disable_local_bottom_half()
    }

    fn guard() -> Self::Guard {
        disable_local_bottom_half()
    }
}

pub(super) struct DisableLocalBottomHalfGuardInPreempt<'a> {
    phantom_data: PhantomData<&'a DisabledPreemptGuard>,
}

impl<'a> Drop for DisableLocalBottomHalfGuardInPreempt<'a> {
    fn drop(&mut self) {
        DISABLE_SOFTIRQ_COUNT.sub_assign(1);
    }
}

pub(super) fn disable_local_bottom_half_with<'a>(
    _preempt_guard: &'a DisabledPreemptGuard,
) -> DisableLocalBottomHalfGuardInPreempt<'a> {
    DISABLE_SOFTIRQ_COUNT.add_assign(1);
    DisableLocalBottomHalfGuardInPreempt {
        phantom_data: PhantomData,
    }
}

#[cfg(ktest)]
mod test {
    use ostd::{
        prelude::*,
        sync::{RwLock, SpinLock},
    };

    use super::*;

    #[ktest]
    fn spinlock_disable_bh() {
        let lock = SpinLock::<(), BottomHalfDisabled>::new(());

        assert!(is_softirq_enabled());

        let guard = lock.lock();
        assert!(!is_softirq_enabled());

        drop(guard);
        assert!(is_softirq_enabled());
    }

    #[ktest]
    fn nested_spin_lock_disable_bh() {
        let lock1 = SpinLock::<(), BottomHalfDisabled>::new(());
        let lock2 = SpinLock::<(), BottomHalfDisabled>::new(());

        assert!(is_softirq_enabled());

        let guard1 = lock1.lock();
        let guard2 = lock2.lock();
        assert!(!is_softirq_enabled());

        drop(guard1);
        assert!(!is_softirq_enabled());

        drop(guard2);
        assert!(is_softirq_enabled());
    }

    #[ktest]
    fn rwlock_disable_bh() {
        let rwlock: RwLock<(), BottomHalfDisabled> = RwLock::new(());

        assert!(is_softirq_enabled());

        let write_guard = rwlock.write();
        assert!(!is_softirq_enabled());

        drop(write_guard);
        assert!(is_softirq_enabled());
    }

    #[ktest]
    fn nested_rwlock_disable_bh() {
        let rwlock: RwLock<(), BottomHalfDisabled> = RwLock::new(());

        assert!(is_softirq_enabled());

        let read_guard1 = rwlock.read();
        let read_guard2 = rwlock.read();
        assert!(!is_softirq_enabled());

        drop(read_guard1);
        assert!(!is_softirq_enabled());

        drop(read_guard2);
        assert!(is_softirq_enabled());
    }

    #[test]
    fn upgradable_rwlock_disable_bh() {
        let rwlock: RwLock<(), BottomHalfDisabled> = RwLock::new(());

        assert!(is_softirq_enabled());

        let upgrade_guard = rwlock.upread();
        assert!(!is_softirq_enabled());

        let write_guard = upgrade_guard.upgrade();
        assert!(!is_softirq_enabled());

        let read_guard = write_guard.downgrade();
        assert!(!is_softirq_enabled());

        drop(read_guard);
        assert!(is_softirq_enabled());
    }
}
