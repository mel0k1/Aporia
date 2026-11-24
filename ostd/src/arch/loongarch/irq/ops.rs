//! Interrupt operations.

pub(crate) fn enable_local() {
    loongArch64::register::crmd::set_ie(true);
}

/// Enables local IRQs and halts the CPU to wait for interrupts.
///
/// This method guarantees that no interrupts can occur in the middle. In other words, IRQs must
/// either have been processed before this method is called, or they must wake the CPU up from the
/// halting state.
pub(crate) fn enable_local_and_halt() {
    loongArch64::register::crmd::set_ie(true);
}

pub(crate) fn disable_local() {
    loongArch64::register::crmd::set_ie(false);
}

/// Disables local IRQs and halts the CPU forever.
pub(crate) fn disable_local_and_halt() -> ! {
    loongArch64::register::crmd::set_ie(false);
    loop {
        unsafe { loongArch64::asm::idle() };
    }
}

pub(crate) fn is_local_enabled() -> bool {
    loongArch64::register::crmd::read().ie()
}
