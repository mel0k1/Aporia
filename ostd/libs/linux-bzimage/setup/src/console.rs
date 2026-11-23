use core::fmt::{self, Write};

use uart_16550::SerialPort;

use crate::sync::Mutex;

struct Stdout {
    serial_port: SerialPort,
}

impl Stdout {
    fn new() -> Self {
        let mut serial_port = unsafe { SerialPort::new(0x3F8) };
        serial_port.init();

        Self { serial_port }
    }
}

impl Write for Stdout {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.serial_port.write_str(s).unwrap();
        Ok(())
    }
}

static STDOUT: Mutex<Option<Stdout>> = Mutex::new(None);

pub(crate) fn print_fmt(args: fmt::Arguments) {
    let mut stdout = STDOUT.lock();

    if let Some(inner) = stdout.as_mut() {
        inner.write_fmt(args).unwrap();
        return;
    }

    let mut inner = Stdout::new();
    inner.write_fmt(args).unwrap();
    *stdout = Some(inner);
}

#[macro_export]
macro_rules! println {
    ($fmt:literal $($arg:tt)*) => {
        $crate::console::print_fmt(format_args!(concat!($fmt, "\n") $($arg)*))
    }
}
