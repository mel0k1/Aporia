//! Configure fast syscall.

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

/// # Safety
///
/// The caller must ensure that:
/// 1. No preemption can occur during the method.
/// 2. `gdt::init_on_cpu` has been called before.
pub(super) unsafe fn init_on_cpu() {
    configure_msrs_racy();

    unsafe {
        Efer::update(|efer| {
            efer.insert(EferFlags::SYSTEM_CALL_EXTENSIONS);
        });
    }
}

/// Configures the kernel's syscall selectors, entry point, and flags mask.
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
    /// Goes to user space with the context, and comes back when a trap occurs.
    ///
    /// The IRQ guard ensures that local IRQs are disabled on entry.
    /// They remain disabled throughout the _kernel-side_ execution of this method.
    /// However, the _user-space_ execution part is still interruptible,
    /// even while the IRQ guard is alive.
    /// The underlying mechanism depends on the CPU architecture.
    /// On x86, `iret` or `sysret` restores the saved user RFLAGS,
    /// whose interrupt flag (IF) is always set.
    ///
    /// On return, the context will be reset to the status before the trap.
    /// Trap reason and error code will be placed at `trap_num` and `error_code`.
    ///
    /// If the trap was triggered by `syscall` instruction, the `trap_num` will be set to `0x100`.
    ///
    /// If `trap_num` is `0x100`, it will go user by `sysret` (`rcx` and `r11` are dropped),
    /// otherwise it will use `iret`.
    pub(in crate::arch) fn run(&mut self, _guard: &DisabledLocalIrqGuard) {
        unsafe { syscall_return(self) };
    }
}
