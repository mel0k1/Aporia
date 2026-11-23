use core::sync::atomic::{AtomicBool, Ordering};

use bitflags::bitflags;
use ostd::{
    arch::{device::io_port::ReadWriteAccess, kernel::ACPI_INFO},
    io::IoPort,
    sync::{LocalIrqDisabled, SpinLock},
};
use spin::Once;

pub(super) static I8042_CONTROLLER: Once<SpinLock<I8042Controller, LocalIrqDisabled>> = Once::new();

pub(super) fn init() -> Result<(), I8042ControllerError> {
    const SELF_TEST_OK: u8 = 0x55;
    const PORT_TEST_OK: u8 = 0x00;

    let mut controller = I8042Controller::new()?;

    controller.wait_and_send_command(Command::DisableFirstPort)?;
    controller.wait_and_send_command(Command::DisableSecondPort)?;

    controller.flush_output_buffer();

    let mut config = controller.read_configuration()?;
    let translation_mode = if config.contains(Configuration::FIRST_PORT_TRANSLATION_ENABLED) {
        super::keyboard::TranslationMode::Hardware
    } else {
        super::keyboard::TranslationMode::Software
    };

    config.remove(
        Configuration::FIRST_PORT_INTERRUPT_ENABLED
            | Configuration::FIRST_PORT_TRANSLATION_ENABLED
            | Configuration::SECOND_PORT_INTERRUPT_ENABLED,
    );
    controller.write_configuration(&config)?;

    if I8042_RESET.load(Ordering::Relaxed) {
        controller.wait_and_send_command(Command::TestController)?;
        let result = controller.wait_and_recv_data()?;
        if result != SELF_TEST_OK {
            return Err(I8042ControllerError::ControllerTestFailed);
        }
        controller.write_configuration(&config)?;
        controller.flush_output_buffer();
    }

    controller.wait_and_send_command(Command::EnableSecondPort)?;
    let has_second_port = config.contains(Configuration::SECOND_PORT_CLOCK_DISABLED)
        && !controller
            .read_configuration()?
            .contains(Configuration::SECOND_PORT_CLOCK_DISABLED);
    controller.wait_and_send_command(Command::DisableSecondPort)?;
    controller.flush_output_buffer();

    if has_second_port {
        controller.wait_and_send_command(Command::TestSecondPort)?;
        let result = controller.wait_and_recv_data()?;
        if result != PORT_TEST_OK {
            return Err(I8042ControllerError::SecondPortTestFailed);
        }
    }

    controller.wait_and_send_command(Command::EnableFirstPort)?;
    if let Err(err) = super::keyboard::init(&mut controller, translation_mode) {
        ostd::warn!("i8042 keyboard initialization failed: {:?}", err);
    } else {
        config.remove(Configuration::FIRST_PORT_CLOCK_DISABLED);
        config.insert(Configuration::FIRST_PORT_INTERRUPT_ENABLED);
        if translation_mode == super::keyboard::TranslationMode::Hardware {
            config.insert(Configuration::FIRST_PORT_TRANSLATION_ENABLED);
        }
    }
    controller.wait_and_send_command(Command::DisableFirstPort)?;
    controller.flush_output_buffer();

    if has_second_port {
        controller.wait_and_send_command(Command::EnableSecondPort)?;
        if let Err(err) = super::mouse::init(&mut controller) {
            ostd::warn!("i8042 mouse initialization failed: {:?}", err);
        } else {
            config.remove(Configuration::SECOND_PORT_CLOCK_DISABLED);
            config.insert(Configuration::SECOND_PORT_INTERRUPT_ENABLED);
        }
        controller.wait_and_send_command(Command::DisableSecondPort)?;
        controller.flush_output_buffer();
    }

    I8042_CONTROLLER.call_once(|| SpinLock::new(controller));
    let mut controller = I8042_CONTROLLER.get().unwrap().lock();
    controller.write_configuration(&config)?;
    controller.flush_output_buffer();

    Ok(())
}

pub(super) struct I8042Controller {
    data_port: IoPort<u8, ReadWriteAccess>,
    status_or_command_port: IoPort<u8, ReadWriteAccess>,
}

impl I8042Controller {
    fn new() -> Result<Self, I8042ControllerError> {
        const DATA_PORT_ADDR: u16 = 0x60;
        const STATUS_OR_COMMAND_PORT_ADDR: u16 = 0x64;

        if !Self::is_present_acpi() {
            if !Self::is_present_cmdline() {
                ostd::info!(
                    "ACPI says i8042 controller is absent; \
                    if it is incorrect, append 'i8042.exist' in cmdline to override it"
                );
                return Err(I8042ControllerError::NotPresent);
            } else {
                ostd::info!(
                    "ACPI says i8042 controller is absent; \
                    however, it is overridden by 'i8042.exist' in cmdline"
                );
            }
        } else {
            ostd::info!("ACPI says i8042 controller is present");
        }

        let controller = Self {
            data_port: IoPort::acquire(DATA_PORT_ADDR).unwrap(),
            status_or_command_port: IoPort::acquire(STATUS_OR_COMMAND_PORT_ADDR).unwrap(),
        };
        Ok(controller)
    }

    fn is_present_acpi() -> bool {
        ACPI_INFO
            .get()
            .unwrap()
            .boot_flags
            .is_some_and(|flags| flags.motherboard_implements_8042())
    }

    fn is_present_cmdline() -> bool {
        I8042_EXIST.load(Ordering::Relaxed)
    }

    fn read_configuration(&mut self) -> Result<Configuration, I8042ControllerError> {
        self.wait_and_send_command(Command::ReadConfiguration)?;
        self.wait_and_recv_data()
            .map(Configuration::from_bits_retain)
    }

    fn write_configuration(&mut self, config: &Configuration) -> Result<(), I8042ControllerError> {
        self.wait_and_send_command(Command::WriteConfiguration)?;
        self.wait_and_send_data(config.bits())
    }

    fn wait_and_send_command(&mut self, command: Command) -> Result<(), I8042ControllerError> {
        spin_wait_until(Timeout::Short, || self.send_command(command).ok())
            .ok_or(I8042ControllerError::OutputBusy)
    }

    fn send_command(&mut self, command: Command) -> Result<(), I8042ControllerError> {
        if !self.read_status().contains(Status::INPUT_BUFFER_IS_FULL) {
            self.write_command(command as u8);
            Ok(())
        } else {
            Err(I8042ControllerError::OutputBusy)
        }
    }

    fn read_status(&self) -> Status {
        Status::from_bits_retain(self.status_or_command_port.read())
    }

    fn write_command(&mut self, command: u8) {
        self.status_or_command_port.write(command);
    }

    pub(super) fn wait_and_send_data(&mut self, data: u8) -> Result<(), I8042ControllerError> {
        spin_wait_until(Timeout::Short, || self.send_data(data).ok())
            .ok_or(I8042ControllerError::OutputBusy)
    }

    pub(super) fn write_to_second_port(&mut self, data: u8) -> Result<(), I8042ControllerError> {
        self.wait_and_send_command(Command::WriteToSecondPort)?;
        self.wait_and_send_data(data)
    }

    pub(super) fn send_data(&mut self, data: u8) -> Result<(), I8042ControllerError> {
        if !self.read_status().contains(Status::INPUT_BUFFER_IS_FULL) {
            self.write_data(data);
            Ok(())
        } else {
            Err(I8042ControllerError::OutputBusy)
        }
    }

    fn write_data(&mut self, data: u8) {
        self.data_port.write(data);
    }

    pub(super) fn wait_and_recv_data(&mut self) -> Result<u8, I8042ControllerError> {
        spin_wait_until(Timeout::Short, || self.receive_data()).ok_or(I8042ControllerError::NoInput)
    }

    pub(super) fn wait_long_and_recv_data(&mut self) -> Result<u8, I8042ControllerError> {
        spin_wait_until(Timeout::Long, || self.receive_data()).ok_or(I8042ControllerError::NoInput)
    }

    pub(super) fn wait_for_specific_data(
        &mut self,
        data: &[u8],
    ) -> Result<u8, I8042ControllerError> {
        spin_wait_until(Timeout::Short, || {
            self.receive_data().filter(|val| data.contains(val))
        })
        .ok_or(I8042ControllerError::NoInput)
    }

    pub(super) fn receive_data(&mut self) -> Option<u8> {
        if self.read_status().contains(Status::OUTPUT_BUFFER_IS_FULL) {
            Some(self.read_data())
        } else {
            None
        }
    }

    fn read_data(&self) -> u8 {
        self.data_port.read()
    }

    fn flush_output_buffer(&mut self) {
        while self.receive_data().is_some() {}
    }

    pub(super) fn reset_cpu(&mut self) {
        if !self.read_status().contains(Status::INPUT_BUFFER_IS_FULL) {
            self.write_command(Command::CpuReset as u8);
        }
    }
}

#[repr(u16)]
#[derive(Clone, Copy, Debug)]
enum Timeout {
    Short = 500,
    Long = 4000,
}

fn spin_wait_until<F, R>(timeout: Timeout, mut f: F) -> Option<R>
where
    F: FnMut() -> Option<R>,
{
    use ostd::arch::{read_tsc, tsc_freq};

    const MSEC_PER_SEC: u64 = 1000;

    if let Some(res) = f() {
        return Some(res);
    }

    let current = read_tsc();
    let distance = tsc_freq() / MSEC_PER_SEC * (timeout as u16 as u64);
    loop {
        if let Some(res) = f() {
            return Some(res);
        }
        if read_tsc().wrapping_sub(current) >= distance {
            return None;
        }
        core::hint::spin_loop();
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum I8042ControllerError {
    NotPresent,
    ControllerTestFailed,
    SecondPortTestFailed,
    OutputBusy,
    NoInput,
    DeviceResetFailed,
    DeviceUnknown,
    DeviceAllocIrqFailed,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug)]
enum Command {
    ReadConfiguration = 0x20,
    WriteConfiguration = 0x60,
    DisableSecondPort = 0xA7,
    EnableSecondPort = 0xA8,
    TestSecondPort = 0xA9,
    TestController = 0xAA,
    DisableFirstPort = 0xAD,
    EnableFirstPort = 0xAE,
    WriteToSecondPort = 0xD4,
    CpuReset = 0xFE,
}

bitflags! {
    struct Configuration: u8 {
        const FIRST_PORT_INTERRUPT_ENABLED   = 1 << 0;
        const SECOND_PORT_INTERRUPT_ENABLED  = 1 << 1;
        const SYSTEM_POST_PASSED             = 1 << 2;
        const FIRST_PORT_CLOCK_DISABLED      = 1 << 4;
        const SECOND_PORT_CLOCK_DISABLED     = 1 << 5;
        const FIRST_PORT_TRANSLATION_ENABLED = 1 << 6;
    }
}

bitflags! {
    struct Status: u8 {
        const OUTPUT_BUFFER_IS_FULL = 1 << 0;
        const INPUT_BUFFER_IS_FULL  = 1 << 1;
        const SYSTEM_FLAG           = 1 << 2;
        const IS_COMMAND            = 1 << 3;
        const TIME_OUT_ERROR        = 1 << 6;
        const PARITY_ERROR          = 1 << 7;
    }
}

static I8042_EXIST: AtomicBool = AtomicBool::new(false);
aster_cmdline::define_flag_param!("i8042.exist", I8042_EXIST);

static I8042_RESET: AtomicBool = AtomicBool::new(false);
aster_cmdline::define_flag_param!("i8042.reset", I8042_RESET);
