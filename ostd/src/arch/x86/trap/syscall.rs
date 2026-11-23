use core::arch::global_asm;

use x86_64::{
    VirtAddr,
    registers::{
        model_specific::{Efer, EferFlags, LStar, SFMask, Star},
        rflags::RFlags,
    },
};

use super::{RawUserContext, gdt};
use crate::{irq::DisabledLocalIrqGuard, mm::PagingConstsTrait};

global_asm!(
    include_str!("syscall.S"),
    USER_CS = const gdt::USER_CS.0,
    USER_SS = const gdt::USER_SS.0,
    ADDRESS_WIDTH = const crate::arch::mm::PagingConsts::ADDRESS_WIDTH,
);

pub(super) unsafe fn init_on_cpu() {
    configure_msrs_racy();

    unsafe {
        Efer::update(|efer| {
            efer.insert(EferFlags::SYSTEM_CALL_EXTENSIONS);
        });
    }
}

pub(in crate::arch) fn configure_msrs(_irq_guard: &DisabledLocalIrqGuard) {
    configure_msrs_racy();
}

fn configure_msrs_racy() {
    const RFLAGS_MASK: u64 = 0x47700;

    unsafe {
        Star::write_raw(gdt::SYSRET_SEL.0, gdt::SYSCALL_SEL.0);
        LStar::write(VirtAddr::new(syscall_entry as *const () as usize as u64));
        SFMask::write(RFlags::from_bits(RFLAGS_MASK).unwrap());
    }
}

unsafe extern "C" {
    unsafe fn syscall_entry();
    unsafe fn syscall_return(regs: &mut RawUserContext);
}

impl RawUserContext {
    pub(in crate::arch) fn run(&mut self, _guard: &DisabledLocalIrqGuard) {
        unsafe { syscall_return(self) };
    }
}
