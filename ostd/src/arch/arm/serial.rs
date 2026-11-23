use core::fmt;

use bitflags::bitflags;
use fdt::node::FdtNode;
use spin::Once;

use super::boot::DEVICE_TREE;
use crate::{
    boot::EarlyCmdline,
    mm::paddr_to_vaddr,
    sync::{LocalIrqDisabled, SpinLock},
};

pub static SERIAL_PORT: Once<SpinLock<Pl011Uart, LocalIrqDisabled>> = Once::new();

pub struct Pl011Uart {
    base: *mut u32,
    node: FdtNode<'static, 'static>,
}

unsafe impl Send for Pl011Uart {}
unsafe impl Sync for Pl011Uart {}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    struct Status: u32 {
        const TXFF = 1 << 5;
        const RXFE = 1 << 4;
    }
}

impl Pl011Uart {
    const OFFSET_UARTDR: usize = 0x000;
    const OFFSET_UARTFR: usize = 0x018;
    const OFFSET_UARTIMSC: usize = 0x038;

    pub(self) unsafe fn new(base: *mut u32, node: FdtNode<'static, 'static>) -> Self {
        Self { base, node }
    }

    pub fn fdt_node(&self) -> FdtNode<'static, 'static> {
        self.node
    }

    pub fn enable_recv_interrupt(&mut self) {
        const RXIM: u32 = 1 << 4;
        const RTIM: u32 = 1 << 6;

        unsafe {
            let uartimsc = self.base.byte_add(Self::OFFSET_UARTIMSC);
            let val = uartimsc.read_volatile();
            uartimsc.write_volatile(val | RXIM | RTIM);
        }
    }

    pub fn send(&mut self, byte: u8) {
        while self.read_status().contains(Status::TXFF) {
            core::hint::spin_loop();
        }
        self.write_data(byte);
    }

    pub fn recv(&mut self) -> Option<u8> {
        if self.read_status().contains(Status::RXFE) {
            None
        } else {
            Some(self.read_data())
        }
    }

    fn write_data(&mut self, data: u8) {
        unsafe {
            self.base
                .byte_add(Self::OFFSET_UARTDR)
                .write_volatile(data as u32);
        }
    }

    fn read_data(&mut self) -> u8 {
        unsafe { self.base.byte_add(Self::OFFSET_UARTDR).read_volatile() as u8 }
    }

    fn read_status(&self) -> Status {
        let raw = unsafe { self.base.byte_add(Self::OFFSET_UARTFR).read_volatile() };
        Status::from_bits_truncate(raw)
    }
}

impl fmt::Write for Pl011Uart {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.as_bytes() {
            self.send(*c);
        }
        Ok(())
    }
}

pub(crate) fn init(early_cmdline: &EarlyCmdline) {
    if !early_cmdline.has_early_console {
        return;
    }

    let Some((base_address, fdt_node)) = lookup_pl011_base_address() else {
        return;
    };

    let pl011 = unsafe { Pl011Uart::new(paddr_to_vaddr(base_address) as *mut u32, fdt_node) };
    SERIAL_PORT.call_once(|| SpinLock::new(pl011));
}

fn lookup_pl011_base_address() -> Option<(usize, FdtNode<'static, 'static>)> {
    let device_tree = DEVICE_TREE.get().unwrap();
    let stdout_path = device_tree
        .find_node("/chosen")?
        .property("stdout-path")?
        .as_str()?;
    let stdout = device_tree.find_node(stdout_path)?;
    if stdout.compatible()?.all().any(|c| c == "arm,pl011") {
        Some((stdout.reg()?.next()?.starting_address as usize, stdout))
    } else {
        None
    }
}
