pub(crate) fn enable_local() {
    loongArch64::register::crmd::set_ie(true);
}

pub(crate) fn enable_local_and_halt() {
    loongArch64::register::crmd::set_ie(true);
}

pub(crate) fn disable_local() {
    loongArch64::register::crmd::set_ie(false);
}

pub(crate) fn disable_local_and_halt() -> ! {
    loongArch64::register::crmd::set_ie(false);
    loop {
        unsafe { loongArch64::asm::idle() };
    }
}

pub(crate) fn is_local_enabled() -> bool {
    loongArch64::register::crmd::read().ie()
}
