mod qemu_isa_debug {

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

pub fn try_poweroff(code: crate::power::ExitCode) {
    qemu_isa_debug::try_exit_qemu(code);
}

pub fn try_restart(_code: crate::power::ExitCode) {}
