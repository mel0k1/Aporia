use spin::Once;

use crate::{arch::irq::disable_local_and_halt, cpu::CpuSet};

#[derive(Clone, Copy)]
pub enum ExitCode {
    Success,
    Failure,
}

static RESTART_HANDLER: Once<fn(ExitCode)> = Once::new();

pub fn inject_restart_handler(handler: fn(ExitCode)) {
    RESTART_HANDLER.call_once(|| handler);
}

pub fn restart(code: ExitCode) -> ! {
    if let Some(handler) = RESTART_HANDLER.get() {
        (handler)(code);
    } else {
        crate::arch::power::try_restart(code);
    }
    crate::error!("Failed to restart the system");

    machine_halt();
}

static POWEROFF_HANDLER: Once<fn(ExitCode)> = Once::new();

pub fn inject_poweroff_handler(handler: fn(ExitCode)) {
    POWEROFF_HANDLER.call_once(|| handler);
}

pub fn poweroff(code: ExitCode) -> ! {
    #[cfg(feature = "coverage")]
    crate::coverage::on_system_exit();

    if let Some(handler) = POWEROFF_HANDLER.get() {
        (handler)(code);
    } else {
        crate::arch::power::try_poweroff(code);
    }
    crate::error!("Failed to power off the system");

    machine_halt();
}

fn machine_halt() -> ! {
    crate::error!("Halting the machine...");

    if let Some(ipi_sender) = crate::smp::IPI_SENDER.get() {
        ipi_sender.inter_processor_call(&CpuSet::new_full(), || disable_local_and_halt());
    }
    disable_local_and_halt();
}
