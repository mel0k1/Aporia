use core::arch::global_asm;

use crate::{arch::cpu::context::GeneralRegs, irq::DisabledLocalIrqGuard, mm::fault::TrapFrameApi};

global_asm!(include_str!("trap.S"));

pub(super) unsafe fn init_on_cpu() {
    loongArch64::register::ecfg::set_vs(0);
    loongArch64::register::eentry::set_eentry(trap_entry as *const () as usize);
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct TrapFrame {
    pub general: GeneralRegs,
    pub prmd: usize,
    pub era: usize,
    pub euen: usize,
}

impl TrapFrameApi for TrapFrame {
    fn set_instruction_pointer(&mut self, ip: usize) {
        self.era = ip;
    }

    fn instruction_pointer(&self) -> usize {
        self.era
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(in crate::arch) struct RawUserContext {
    pub(in crate::arch) general: GeneralRegs,
    pub(in crate::arch) prmd: usize,
    pub(in crate::arch) era: usize,
    pub(in crate::arch) euen: usize,
}

impl Default for RawUserContext {
    fn default() -> Self {
        Self {
            general: GeneralRegs::default(),
            prmd: 0b111,
            era: 0,
            euen: 0,
        }
    }
}

impl RawUserContext {
    pub(in crate::arch) fn run(&mut self, _guard: &DisabledLocalIrqGuard) {
        unsafe { run_user(self) };
    }
}

unsafe extern "C" {
    unsafe fn trap_entry();
    unsafe fn run_user(regs: &mut RawUserContext);
}
