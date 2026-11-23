use core::fmt;

use spin::Once;

use crate::{
    boot::EarlyCmdline,
    sync::{LocalIrqDisabled, SpinLock},
};

pub(crate) static SERIAL_PORT: Once<SpinLock<SbiSerial, LocalIrqDisabled>> =
    Once::initialized(SpinLock::new(SbiSerial::new()));

pub(crate) struct SbiSerial {
    _private: (),
}

impl SbiSerial {
    const fn new() -> Self {
        Self { _private: () }
    }
}

impl fmt::Write for SbiSerial {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.as_bytes() {
            sbi_rt::console_write_byte(*c);
        }
        Ok(())
    }
}

pub(crate) fn init(_early_cmdline: &EarlyCmdline) {}
