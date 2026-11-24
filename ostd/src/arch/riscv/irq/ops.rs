//! Interrupt operations.

pub(crate) fn enable_local() {
    unsafe { riscv::interrupt::enable() }
}

/// Enables local IRQs and halts the CPU to wait for interrupts.
///
/// This method guarantees that no interrupts can occur in the middle. In other words, IRQs must
/// either have been processed before this method is called, or they must wake the CPU up from the
/// halting state.
pub(crate) fn enable_local_and_halt() {
    riscv::asm::wfi();

    unsafe { riscv::interrupt::enable() }
}

pub(crate) fn disable_local() {
    riscv::interrupt::disable();
}

/// Disables local IRQs and halts the CPU forever.
pub(crate) fn disable_local_and_halt() -> ! {
    riscv::interrupt::disable();
    loop {
        riscv::asm::wfi();
    }
}

pub(crate) fn is_local_enabled() -> bool {
    riscv::register::sstatus::read().sie()
}
