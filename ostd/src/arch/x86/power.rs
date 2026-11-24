//! Power management.

mod qemu_isa_debug {
    //! The isa-debug-exit device in QEMU.
    //!
    //! Reference: <https://elixir.bootlin.com/qemu/v10.1.2/source/hw/misc/debugexit.c>

    use spin::Once;

    use crate::{arch::device::io_port::WriteOnlyAccess, io::IoPort, power::ExitCode};

    const EXIT_SUCCESS: u32 = 0x10;
    const EXIT_FAILURE: u32 = 0x20;

    static DEBUG_EXIT_PORT: Once<IoPort<u32, WriteOnlyAccess>> = Once::new();

    pub(super) fn try_exit_qemu(code: ExitCode) {
        let value = match code {
            ExitCode::Success => EXIT_SUCCESS,
            ExitCode::Failure => EXIT_FAILURE,
        };

        if let Some(port) = DEBUG_EXIT_PORT.get() {
            port.write(value);
        }
    }

    pub(super) fn init() {
        const DEBUG_EXIT_PORT_NUM: u16 = 0xF4;

        let debug_exit_port = IoPort::acquire(DEBUG_EXIT_PORT_NUM).unwrap();

        DEBUG_EXIT_PORT.call_once(|| debug_exit_port);
    }
}

pub(super) fn init() {
    use super::cpu::cpuid;

    if !cpuid::query_if_running_in_qemu() {
        return;
    }

    crate::info!("QEMU hypervisor detected, assuming that the isa-debug-exit device exists");

    qemu_isa_debug::init();
}

/// Attempts to power off the system using an architecture-specific mechanism.
///
/// On x86, this function attempts to power off the system through QEMU's ISA debug-exit device if
/// QEMU was detected during initialization. Otherwise, it does nothing and returns.
pub fn try_poweroff(code: crate::power::ExitCode) {
    qemu_isa_debug::try_exit_qemu(code);
}

/// Attempts to restart the system using an architecture-specific mechanism.
///
/// On x86, this function currently does nothing and returns.
pub fn try_restart(_code: crate::power::ExitCode) {}
