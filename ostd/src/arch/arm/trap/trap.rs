use core::arch::{asm, global_asm};

use crate::{arch::cpu::context::GeneralRegs, irq::DisabledLocalIrqGuard, mm::fault::TrapFrameApi};

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::arch) struct RawUserContext {
    pub(in crate::arch) trap_num: usize,
    pub(in crate::arch) __reserved: usize,
    pub(in crate::arch) elr: usize,
    pub(in crate::arch) spsr: usize,
    pub(in crate::arch) sp: usize,
    pub(in crate::arch) tpidr: usize,
    pub(in crate::arch) general: GeneralRegs,
}

global_asm!(include_str!("trap.S"));

pub(super) unsafe fn init_on_cpu() {
    unsafe {
        asm!("msr vbar_el1, {}", in(reg) __vectors as *const () as usize);
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct TrapFrame {
    pub trap_num: usize,
    pub __reserved: usize,
    pub elr: usize,
    pub spsr: usize,
    pub sp: usize,
    pub tpidr: usize,
    pub general: GeneralRegs,
}

impl TrapFrameApi for TrapFrame {
    fn set_instruction_pointer(&mut self, ip: usize) {
        self.elr = ip;
    }

    fn instruction_pointer(&self) -> usize {
        self.elr
    }
}

impl RawUserContext {
    pub(in crate::arch) fn run(&mut self, _guard: &DisabledLocalIrqGuard) {
        unsafe { run_user(self) };
    }
}

unsafe extern "C" {
    unsafe fn __vectors();
    unsafe fn run_user(regs: &mut RawUserContext);
}
