//! Interrupt operations.

use core::arch::asm;

pub(crate) fn enable_local() {
    unsafe { asm!("msr daifclr, 0b0010") };
}

/// Enables local IRQs and halts the CPU to wait for interrupts.
///
/// This method guarantees that no interrupts can occur in the middle. In other words, IRQs must
/// either have been processed before this method is called, or they must wake the CPU up from the
/// halting state.
pub(crate) fn enable_local_and_halt() {
    unsafe { asm!("wfi") };

    enable_local();
}

pub(crate) fn disable_local() {
    unsafe { asm!("msr daifset, 0b0010") };
}

/// Disables local IRQs and halts the CPU forever.
pub(crate) fn disable_local_and_halt() -> ! {
    disable_local();
    loop {
        unsafe { asm!("wfi") };
    }
}

pub(in crate::arch) const PSTATE_I: usize = 1 << 7;

pub(crate) fn is_local_enabled() -> bool {
    let daif: usize;

    unsafe { asm!("mrs {}, daif", out(reg) daif) };

    daif & PSTATE_I == 0
}
