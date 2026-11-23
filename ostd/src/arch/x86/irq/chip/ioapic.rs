use bit_field::BitField;

use crate::{
    Error, Result, info,
    io::{IoMem, IoMemAllocatorBuilder, Sensitive},
    irq::IrqLine,
    mm::HasPaddr,
};

pub(super) struct IoApic {
    access: IoApicAccess,
    interrupt_base: u32,
    max_redirection_entry: u8,
}

impl IoApic {
    pub(super) unsafe fn new(
        base_address: usize,
        base_interrupt: u32,
        io_mem_builder: &IoMemAllocatorBuilder,
    ) -> Option<Self> {
        let mut access = unsafe { IoApicAccess::new(base_address, io_mem_builder) };

        let reg_id = unsafe { access.read(IoApicAccess::IOAPICID) };
        let reg_ver = unsafe { access.read(IoApicAccess::IOAPICVER) };
        let reg_arb = unsafe { access.read(IoApicAccess::IOAPICARB) };
        if reg_id == u32::MAX && reg_ver == u32::MAX && reg_arb == u32::MAX {
            crate::warn!(
                "IOAPIC {:#x} registers return all ones, skipping",
                base_address
            );
            return None;
        }

        let max_redirection_entry = access.max_redirection_entry();
        if base_interrupt
            .checked_add(max_redirection_entry as u32)
            .is_none()
        {
            crate::warn!(
                "IOAPIC {:#x} GSI range overflows ({}+{}), skipping",
                base_address,
                base_interrupt,
                max_redirection_entry
            );
            return None;
        }

        info!(
            "IOAPIC found at {:#x}, ID {}, version {}, interrupt base {}, interrupt count {}",
            base_address,
            access.id(),
            access.version(),
            base_interrupt,
            max_redirection_entry,
        );

        let mut ioapic = Self {
            access,
            interrupt_base: base_interrupt,
            max_redirection_entry,
        };

        for index in 0..=max_redirection_entry {
            ioapic.disable(index).unwrap();
        }

        Some(ioapic)
    }

    pub(super) fn enable(&mut self, index: u8, irq: &IrqLine) -> Result<()> {
        if index > self.max_redirection_entry {
            return Err(Error::InvalidArgs);
        }

        let value = unsafe { self.access.read(IoApicAccess::IOREDTBL + 2 * index) };
        if value.get_bits(0..8) as u8 != 0 {
            return Err(Error::AccessDenied);
        }

        if let Some(remapping_index) = irq.remapping_index() {
            let mut value: u64 = irq.num() as u64 | 0x1_0000_0000_0000;

            value |= ((remapping_index & 0x8000) >> 4) as u64;
            value |= (remapping_index as u64 & 0x7FFF) << 49;

            unsafe {
                self.access.write(
                    IoApicAccess::IOREDTBL + 2 * index,
                    value.get_bits(0..32) as u32,
                );
                self.access.write(
                    IoApicAccess::IOREDTBL + 2 * index + 1,
                    value.get_bits(32..64) as u32,
                );
            }
        } else {
            unsafe {
                self.access
                    .write(IoApicAccess::IOREDTBL + 2 * index, irq.num() as u32);
                self.access.write(IoApicAccess::IOREDTBL + 2 * index + 1, 0);
            }
        }

        Ok(())
    }

    pub(super) fn disable(&mut self, index: u8) -> Result<()> {
        if index > self.max_redirection_entry {
            return Err(Error::InvalidArgs);
        }

        unsafe {
            self.access
                .write(IoApicAccess::IOREDTBL + 2 * index, 1 << 16);
            self.access.write(IoApicAccess::IOREDTBL + 2 * index + 1, 0);
        }

        Ok(())
    }

    pub(super) fn interrupt_base(&self) -> u32 {
        self.interrupt_base
    }

    pub(super) fn interrupt_end(&self) -> u32 {
        self.interrupt_base + self.max_redirection_entry as u32
    }

    pub(super) fn address(&self) -> usize {
        self.access.io_mem.paddr()
    }
}

struct IoApicAccess {
    io_mem: IoMem<Sensitive>,
}

impl IoApicAccess {
    const MMIO_REGSEL: usize = 0x00;
    const MMIO_WIN: usize = 0x10;
    const MMIO_SIZE: usize = crate::mm::PAGE_SIZE;

    pub(self) const IOAPICID: u8 = 0x00;
    pub(self) const IOAPICVER: u8 = 0x01;
    pub(self) const IOAPICARB: u8 = 0x02;
    pub(self) const IOREDTBL: u8 = 0x10;

    pub(self) unsafe fn new(base_address: usize, io_mem_builder: &IoMemAllocatorBuilder) -> Self {
        let io_mem = io_mem_builder.reserve(
            base_address..(base_address + Self::MMIO_SIZE),
            crate::mm::CachePolicy::Uncacheable,
        );

        Self { io_mem }
    }

    pub(self) unsafe fn read(&mut self, register: u8) -> u32 {
        unsafe {
            self.io_mem
                .write_once(Self::MMIO_REGSEL, &(register as u32));
            self.io_mem.read_once(Self::MMIO_WIN)
        }
    }

    pub(self) unsafe fn write(&mut self, register: u8, data: u32) {
        unsafe {
            self.io_mem
                .write_once(Self::MMIO_REGSEL, &(register as u32));
            self.io_mem.write_once(Self::MMIO_WIN, &data);
        }
    }

    pub(self) fn id(&mut self) -> u8 {
        unsafe { self.read(Self::IOAPICID).get_bits(24..28) as u8 }
    }

    pub(self) fn version(&mut self) -> u8 {
        unsafe { self.read(Self::IOAPICVER).get_bits(0..8) as u8 }
    }

    pub(self) fn max_redirection_entry(&mut self) -> u8 {
        unsafe { self.read(Self::IOAPICVER).get_bits(16..24) as u8 }
    }
}
