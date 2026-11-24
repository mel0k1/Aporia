use ostd::{
    arch::{device::io_port::WriteOnlyAccess, kernel::ACPI_INFO},
    io::IoPort,
    power::ExitCode,
};
use spin::Once;

static ACPI_RESET_PORT_AND_VAL: Once<(IoPort<u8, WriteOnlyAccess>, u8)> = Once::new();

fn try_acpi_reset(_code: ExitCode) {
    if let Some((port, val)) = ACPI_RESET_PORT_AND_VAL.get() {
        port.write(*val);
    }
}

crate::register_restart_handler!(try_acpi_reset, crate::power::Priority::DEFAULT);

crate::register_poweroff_handler!(
    ostd::arch::power::try_poweroff,
    crate::power::Priority::HIGH
);

pub(super) fn init() {
    let acpi_info = ACPI_INFO.get().unwrap();

    if let Some((reset_port_num, reset_val)) = acpi_info.reset_port_and_val {
        if let Ok(reset_port) = IoPort::acquire(reset_port_num) {
            ACPI_RESET_PORT_AND_VAL.call_once(move || (reset_port, reset_val));
        } else {
            ostd::warn!("The reset port from ACPI is not available");
        }
    }
}
