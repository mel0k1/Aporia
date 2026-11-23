use core::cell::{Cell, RefCell};

use ostd::irq::DisabledLocalIrqGuard;

#[derive(Debug)]
pub(crate) struct CpuSync<R> {
    location: Cell<CanonicalValueLocation>,
    reg: RefCell<R>,
}

impl<R: UserReg> CpuSync<R> {
    pub(super) const fn new(reg: R) -> Self {
        Self {
            location: Cell::new(CanonicalValueLocation::InMemory),
            reg: RefCell::new(reg),
        }
    }

    pub(crate) fn get(&self) -> R
    where
        R: Clone,
    {
        if self.location.get() == CanonicalValueLocation::OnCpu {
            self.reg.borrow_mut().save_from_cpu();
            self.location.set(CanonicalValueLocation::Both);
        }
        self.reg.borrow().clone()
    }

    pub(crate) fn set(&self, new_reg: R) {
        *self.reg.borrow_mut() = new_reg;
        if self.location.get() != CanonicalValueLocation::InMemory {
            self.location.set(CanonicalValueLocation::InMemory);
        }
    }

    pub(super) fn before_schedule(&self, guard: &DisabledLocalIrqGuard) {
        let loc = self.location.get();
        if loc == CanonicalValueLocation::OnCpu {
            self.reg.borrow_mut().save_from_cpu_with_irq_disabled(guard);
        }
        if loc != CanonicalValueLocation::InMemory {
            self.location.set(CanonicalValueLocation::InMemory);
        }
    }

    pub(super) fn before_user_exec(&self, guard: &DisabledLocalIrqGuard) {
        if self.location.get() == CanonicalValueLocation::InMemory {
            self.reg.borrow().restore_to_cpu_with_irq_disabled(guard);
        }
        self.location.set(CanonicalValueLocation::OnCpu);
    }
}

pub(crate) trait UserReg {
    fn save_from_cpu(&mut self);

    fn save_from_cpu_with_irq_disabled(&mut self, _guard: &DisabledLocalIrqGuard) {
        self.save_from_cpu();
    }

    fn restore_to_cpu(&self);

    fn restore_to_cpu_with_irq_disabled(&self, _guard: &DisabledLocalIrqGuard) {
        self.restore_to_cpu();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CanonicalValueLocation {
    InMemory,
    OnCpu,
    Both,
}

impl UserReg for ostd::arch::cpu::context::FpuContext {
    fn save_from_cpu(&mut self) {
        self.save();
    }

    fn restore_to_cpu(&self) {
        self.load();
    }
}

#[cfg(target_arch = "x86_64")]
impl UserReg for ostd::arch::cpu::context::FsBase {
    fn save_from_cpu(&mut self) {
        self.save();
    }

    fn restore_to_cpu(&self) {
        self.load();
    }
}

#[cfg(target_arch = "x86_64")]
impl UserReg for ostd::arch::cpu::context::GsBase {
    fn save_from_cpu(&mut self) {
        let guard = ostd::irq::disable_local();
        self.save(&guard);
    }

    fn save_from_cpu_with_irq_disabled(&mut self, guard: &DisabledLocalIrqGuard) {
        self.save(guard);
    }

    fn restore_to_cpu(&self) {
        let guard = ostd::irq::disable_local();
        self.load(&guard);
    }

    fn restore_to_cpu_with_irq_disabled(&self, guard: &DisabledLocalIrqGuard) {
        self.load(guard)
    }
}
