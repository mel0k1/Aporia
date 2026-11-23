use core::arch::{asm, global_asm};

use crate::{
    arch::cpu::{
        context::GeneralRegs,
        extension::{IsaExtensions, has_extensions},
    },
    irq::DisabledLocalIrqGuard,
    mm::fault::TrapFrameApi,
};

pub(in crate::arch) const SSTATUS_FS_MASK: usize = 0b11 << 13;
pub(in crate::arch) const SSTATUS_SUM: usize = 0b1 << 18;

global_asm!(
    include_str!("trap.S"),
    SSTATUS_FS_MASK = const SSTATUS_FS_MASK,
    SSTATUS_SUM = const SSTATUS_SUM
);

pub(super) unsafe fn init_on_cpu() {
    unsafe {
        asm!("csrw sscratch, zero");
        asm!("csrw stvec, {}", in(reg) trap_entry as *const () as usize);
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct TrapFrame {
    pub general: GeneralRegs,
    pub sstatus: usize,
    pub sepc: usize,
}

impl TrapFrameApi for TrapFrame {
    fn set_instruction_pointer(&mut self, ip: usize) {
        self.sepc = ip;
    }

    fn instruction_pointer(&self) -> usize {
        self.sepc
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(in crate::arch) struct RawUserContext {
    pub(in crate::arch) general: GeneralRegs,
    pub(in crate::arch) sstatus: usize,
    pub(in crate::arch) sepc: usize,
}

impl Default for RawUserContext {
    fn default() -> Self {
        let sstatus = if has_extensions(IsaExtensions::F)
            || has_extensions(IsaExtensions::D)
            || has_extensions(IsaExtensions::Q)
        {
            const SSTATUS_FS_INITIAL: usize = 0b01 << 13;
            SSTATUS_FS_INITIAL
        } else {
            0
        };

        Self {
            general: GeneralRegs::default(),
            sstatus,
            sepc: 0,
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
