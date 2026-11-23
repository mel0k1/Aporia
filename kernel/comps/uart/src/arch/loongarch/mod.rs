use ostd::arch::serial::SERIAL_PORT;

use crate::{
    alloc::string::ToString,
    console::{Uart, UartConsole},
};

pub(super) fn init() {
    let Some(uart) = SERIAL_PORT.get() else {
        return;
    };

    let uart_console = UartConsole::new(uart);

    aster_console::register_device(
        aster_console::UART_CONSOLE_NAME.to_string(),
        uart_console.clone(),
    );

    let _ = || uart_console.trigger_input_callbacks();
    let _ = || uart.flush();

    ostd::info!("Registered NS16550A as a console");
}
