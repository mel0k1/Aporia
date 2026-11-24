use crate::{
    arch::device::io_port::{ReadWriteAccess, WriteOnlyAccess},
    info,
    io::{IoPort, sensitive_io_port},
};

sensitive_io_port! {
    unsafe {
        static MASTER_CMD: IoPort<u8, WriteOnlyAccess> = IoPort::new(0x20);
        static MASTER_DATA: IoPort<u8, ReadWriteAccess> = IoPort::new(0x21);
        static SLAVE_CMD: IoPort<u8, WriteOnlyAccess> = IoPort::new(0xA0);
        static SLAVE_DATA: IoPort<u8, WriteOnlyAccess> = IoPort::new(0xA1);
    }
}

const IRQ_OFFSET: u8 = 0x20;

/// Probes whether a legacy 8259A PIC is present.
///
/// Reference: <https://elixir.bootlin.com/linux/v7.0/source/arch/x86/kernel/i8259.c#L306>
pub(super) fn probe_legacy_pic() -> bool {
    const PROBE_VAL: u8 = 0xFB;

    SLAVE_DATA.write(0xFF);

    MASTER_DATA.write(PROBE_VAL);

    MASTER_DATA.read() == PROBE_VAL
}

/// Initializes and disables the 8259 Programmable Interrupt Controller (PIC).
pub(super) fn init_and_disable() {
    info!("Initializing PIC as disabled");

    set_mask(0xff, 0xff);
}

fn set_mask(master_mask: u8, slave_mask: u8) {
    MASTER_CMD.write(0x11);
    SLAVE_CMD.write(0x11);

    MASTER_DATA.write(IRQ_OFFSET);

    SLAVE_DATA.write(IRQ_OFFSET + 0x08);

    MASTER_DATA.write(4);
    SLAVE_DATA.write(2);

    MASTER_DATA.write(1);
    SLAVE_DATA.write(1);

    MASTER_DATA.write(master_mask);
    SLAVE_DATA.write(slave_mask);
}
