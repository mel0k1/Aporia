#![expect(dead_code)]

use crate::{
    arch::{
        device::io_port::WriteOnlyAccess,
        irq::{IRQ_CHIP, MappedIrqLine},
    },
    io::{IoPort, sensitive_io_port},
    irq::IrqLine,
    timer::TIMER_FREQ,
};

#[repr(u8)]
pub(in crate::arch) enum OperatingMode {
    InterruptOnTerminalCount = 0b000,
    OneShotHardwareRetriggerable = 0b001,
    RateGenerator = 0b010,
    SquareWaveGenerator = 0b011,
    SoftwareTriggeredStrobe = 0b100,
    HardwareTriggeredStrobe = 0b101,
}

#[repr(u8)]
enum AccessMode {
    LatchCountValueCommand = 0b00,
    LowByteOnly = 0b01,
    HighByteOnly = 0b10,
    LowAndHighByte = 0b11,
}

#[repr(u8)]
enum Channel {
    Channel0 = 0b00,
    Channel1 = 0b01,
    Channel2 = 0b10,
    ReadBackCommand = 0b11,
}

sensitive_io_port! {
    unsafe {
        static CHANNEL0_PORT: IoPort<u8, WriteOnlyAccess> = IoPort::new(0x40);

        static CHANNEL1_PORT: IoPort<u8, WriteOnlyAccess> = IoPort::new(0x41);

        static CHANNEL2_PORT: IoPort<u8, WriteOnlyAccess> = IoPort::new(0x42);

        static MODE_COMMAND_PORT: IoPort<u8, WriteOnlyAccess> = IoPort::new(0x43);
    }
}

const TIMER_RATE: u32 = 1193182;
const TIMER_INTERRUPT: u8 = 0;

pub(in crate::arch) fn init(operating_mode: OperatingMode) {
    MODE_COMMAND_PORT.write(
        ((operating_mode as u8) << 1)
            | ((AccessMode::LowAndHighByte as u8) << 4)
            | ((Channel::Channel0 as u8) << 6),
    );

    const CYCLE: u32 = TIMER_RATE / TIMER_FREQ as u32;
    CHANNEL0_PORT.write((CYCLE & 0xFF) as _);
    CHANNEL0_PORT.write((CYCLE >> 8) as _);
}

pub(in crate::arch) fn enable_interrupt(irq_line: IrqLine) -> MappedIrqLine {
    IRQ_CHIP
        .get()
        .unwrap()
        .map_isa_pin_to(irq_line, TIMER_INTERRUPT)
        .unwrap()
}
