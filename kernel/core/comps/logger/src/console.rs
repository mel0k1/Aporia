use alloc::{collections::btree_map::BTreeMap, fmt, string::String, sync::Arc};
use core::fmt::Write;

use aster_console::AnyConsoleDevice;
use ostd::sync::{LocalIrqDisabled, SpinLockGuard};

pub fn _print(args: fmt::Arguments) {
    let devices = aster_console::all_devices_lock();

    struct Printer<'a>(
        SpinLockGuard<'a, BTreeMap<String, Arc<dyn AnyConsoleDevice>>, LocalIrqDisabled>,
    );
    impl Write for Printer<'_> {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            if self.0.is_empty() {
                ostd::early_print!("{}", s);
            } else {
                for console in self.0.values() {
                    console.send(s.as_bytes());
                }
            }
            Ok(())
        }
    }

    Printer(devices).write_fmt(args).unwrap();
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {{
        $crate::_print(format_args!($($arg)*));
    }};
}

#[macro_export]
macro_rules! println {
    () => {
        $crate::print!("\n")
    };
    ($($arg:tt)*) => {{
        $crate::_print(::core::format_args_nl!($($arg)*));
    }};
}
