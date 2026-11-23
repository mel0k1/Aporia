pub(crate) fn enable_local() {
    unsafe { riscv::interrupt::enable() }
}

pub(crate) fn enable_local_and_halt() {
    riscv::asm::wfi();
    unsafe { riscv::interrupt::enable() }
}

pub(crate) fn disable_local() {
    riscv::interrupt::disable();
}

pub(crate) fn disable_local_and_halt() -> ! {
    riscv::interrupt::disable();
    loop {
        riscv::asm::wfi();
    }
}

pub(crate) fn is_local_enabled() -> bool {
    riscv::register::sstatus::read().sie()
}
