use crate::{
    irq::{DisabledLocalIrqGuard, disable_local},
    task::{DisabledPreemptGuard, atomic_mode::AsAtomicModeGuard, disable_preempt},
};

pub trait SpinGuardian {
    type Guard: AsAtomicModeGuard + GuardTransfer;
    type ReadGuard: AsAtomicModeGuard + GuardTransfer;

    fn guard() -> Self::Guard;
    fn read_guard() -> Self::ReadGuard;
}

pub trait GuardTransfer {
    fn transfer_to(&mut self) -> Self;
}

pub enum PreemptDisabled {}

impl SpinGuardian for PreemptDisabled {
    type Guard = DisabledPreemptGuard;
    type ReadGuard = DisabledPreemptGuard;

    fn guard() -> Self::Guard {
        disable_preempt()
    }
    fn read_guard() -> Self::Guard {
        disable_preempt()
    }
}

pub enum LocalIrqDisabled {}

impl SpinGuardian for LocalIrqDisabled {
    type Guard = DisabledLocalIrqGuard;
    type ReadGuard = DisabledLocalIrqGuard;

    fn guard() -> Self::Guard {
        disable_local()
    }
    fn read_guard() -> Self::Guard {
        disable_local()
    }
}

pub enum WriteIrqDisabled {}

impl SpinGuardian for WriteIrqDisabled {
    type Guard = DisabledLocalIrqGuard;
    type ReadGuard = DisabledPreemptGuard;

    fn guard() -> Self::Guard {
        disable_local()
    }
    fn read_guard() -> Self::ReadGuard {
        disable_preempt()
    }
}
