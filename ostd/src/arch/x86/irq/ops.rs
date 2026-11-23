use x86_64::registers::rflags::{self, RFlags};

pub(crate) fn enable_local() {
    x86_64::instructions::interrupts::enable();
    x86_64::instructions::nop();
}

pub(crate) fn enable_local_and_halt() {
    unsafe { core::arch::asm!("sti", "hlt", options(nomem, nostack, preserves_flags),) };
}

pub(crate) fn disable_local() {
    x86_64::instructions::interrupts::disable();
}

pub(crate) fn disable_local_and_halt() -> ! {
    x86_64::instructions::interrupts::disable();
    loop {
        x86_64::instructions::hlt();
    }
}

pub(crate) fn is_local_enabled() -> bool {
    (rflags::read_raw() & RFlags::INTERRUPT_FLAG.bits()) != 0
}
