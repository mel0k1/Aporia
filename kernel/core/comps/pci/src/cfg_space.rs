use bitflags::bitflags;
use ostd::{
    Error, Result,
    arch::device::io_port::{PortRead, PortWrite},
    io::IoMem,
    mm::{PodOnce, VmIoOnce},
};
use spin::Once;

use super::PciDeviceLocation;

#[repr(u16)]
pub enum PciCommonCfgOffset {
    VendorId = 0x00,
    DeviceId = 0x02,
    Command = 0x04,
    Status = 0x06,
    RevisionId = 0x08,
    ProgIf = 0x09,
    SubclassCode = 0x0A,
    BaseClassCode = 0x0B,
    CacheLineSize = 0x0C,
    LatencyTimer = 0x0D,
    HeaderType = 0x0E,
    Bist = 0x0F,
    InterruptLine = 0x3C,
    InterruptPin = 0x3D,
}

#[repr(u16)]
pub enum PciGeneralDeviceCfgOffset {
    Bar0 = 0x10,
    Bar1 = 0x14,
    Bar2 = 0x18,
    Bar3 = 0x1C,
    Bar4 = 0x20,
    Bar5 = 0x24,
    CardbusCisPtr = 0x28,
    SubsystemVendorId = 0x2C,
    SubsystemId = 0x2E,
    XromBar = 0x30,
    CapabilitiesPointer = 0x34,
    MinGrant = 0x3E,
    MaxLatency = 0x3F,
}

#[repr(u16)]
pub enum PciBridgeCfgOffset {
    Bar0 = 0x10,
    Bar1 = 0x14,
    PrimaryBusNumber = 0x18,
    SecondaryBusNumber = 0x19,
    SubordinateBusNumber = 0x1A,
    SecondaryLatencyTimer = 0x1B,
    IoBase = 0x1C,
    IoLimit = 0x1D,
    SecondaryStatus = 0x1E,
    MemoryBase = 0x20,
    MemoryLimit = 0x22,
    PrefetchableMemoryBase = 0x24,
    PrefetchableMemoryLimit = 0x26,
    PrefetchableMemoryBaseUpper32 = 0x28,
    PrefetchableMemoryLimitUpper32 = 0x2C,
    IoBaseUpper16 = 0x30,
    IoLimitUpper16 = 0x32,
    CapabilitiesPointer = 0x34,
    BridgeControl = 0x3E,
}

#[repr(u16)]
pub enum PciCardbusBridgeCfgOffset {
    CardbusSocketExcaBaseAddress = 0x10,
    CapabilitiesPointer = 0x14,
    SecondaryStatus = 0x16,
    CardbusLatencyTimer = 0x17,
    MemoryBase0 = 0x18,
    MemoryLimit0 = 0x1C,
    MemoryBase1 = 0x20,
    MemoryLimit1 = 0x24,
    IoBase0 = 0x28,
    IoLimit0 = 0x2C,
    IoBase1 = 0x30,
    IoLimit1 = 0x34,
    BridgeControl = 0x3E,
    SubsystemDeviceId = 0x40,
    SubsystemVendorId = 0x42,
    LegacyModeBaseAddress = 0x44,
}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub struct Command: u16 {
        const IO_SPACE                 = 1 << 0;
        const MEMORY_SPACE             = 1 << 1;
        const BUS_MASTER               = 1 << 2;
        const SPECIAL_CYCLES           = 1 << 3;
        const MWI_ENABLE               = 1 << 4;
        const VGA_PALETTE_SNOOP        = 1 << 5;
        const PARITY_ERROR_RESPONSE    = 1 << 6;
        const SERR_ENABLE              = 1 << 8;
        const FAST_BACK_TO_BACK_ENABLE = 1 << 9;
        const INTERRUPT_DISABLE        = 1 << 10;
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub struct Status: u16 {
        const INTERRUPT_STATUS          = 1 << 3;
        const CAPABILITIES_LIST         = 1 << 4;
        const MHZ66_CAPABLE             = 1 << 5;
        const FAST_BACK_TO_BACK_CAPABLE = 1 << 7;
        const MASTER_DATA_PARITY_ERROR  = 1 << 8;
        const DEVSEL_MEDIUM_TIMING      = 1 << 9;
        const DEVSEL_SLOW_TIMING        = 1 << 10;
        const SIGNALED_TARGET_ABORT     = 1 << 11;
        const RECEIVED_TARGET_ABORT     = 1 << 12;
        const RECEIVED_MASTER_ABORT     = 1 << 13;
        const SIGNALED_SYSTEM_ERROR     = 1 << 14;
        const DETECTED_PARITY_ERROR     = 1 << 15;
    }
}

#[derive(Debug)]
pub enum Bar {
    Memory(MemoryBar),
    Io(IoBar),
}

impl Bar {
    pub fn acquire(&self) -> Result<BarAccess> {
        match self {
            Self::Memory(mem_bar) => mem_bar.acquire().cloned().map(BarAccess::Memory),
            Self::Io(_) => Ok(BarAccess::Io),
        }
    }

    pub fn access(&self) -> Option<BarAccess> {
        match self {
            Self::Memory(mem_bar) => mem_bar.access().cloned().map(BarAccess::Memory),
            Self::Io(_) => Some(BarAccess::Io),
        }
    }

    pub(super) fn new(location: PciDeviceLocation, index: u8) -> Result<Self> {
        if index >= 6 {
            return Err(Error::InvalidArgs);
        }

        let offset = index as u16 * 4 + PciGeneralDeviceCfgOffset::Bar0 as u16;
        let raw = location.read32(offset);

        let result = if raw & 1 == 0 {
            Self::Memory(MemoryBar::new(&location, index, raw)?)
        } else {
            Self::Io(IoBar::new(&location, index, raw)?)
        };
        Ok(result)
    }
}

#[derive(Clone, Debug)]
pub enum BarAccess {
    Memory(IoMem),
    Io,
}

impl BarAccess {
    pub fn read_once<T: PodOnce + PortRead>(&self, offset: usize) -> Result<T> {
        match self {
            Self::Memory(io_mem) => io_mem.read_once(offset),
            Self::Io => Err(Error::IoError),
        }
    }

    pub fn write_once<T: PodOnce + PortWrite>(&self, offset: usize, value: T) -> Result<()> {
        match self {
            Self::Memory(io_mem) => io_mem.write_once(offset, &value),
            Self::Io => Err(Error::IoError),
        }
    }
}

#[derive(Debug)]
pub struct MemoryBar {
    base: u64,
    size: u64,
    prefetchable: bool,
    address_length: AddrLen,
    io_memory: Once<IoMem>,
}

impl MemoryBar {
    pub fn address_length(&self) -> AddrLen {
        self.address_length
    }

    pub fn prefetchable(&self) -> bool {
        self.prefetchable
    }

    pub fn base(&self) -> u64 {
        self.base
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn acquire(&self) -> Result<&IoMem> {
        self.io_memory.try_call_once(|| {
            IoMem::acquire((self.base as usize)..((self.base + self.size) as usize))
        })
    }

    pub fn access(&self) -> Option<&IoMem> {
        self.io_memory.get()
    }

    fn new(location: &PciDeviceLocation, index: u8, raw: u32) -> Result<Self> {
        debug_assert_eq!(raw & 1, 0);

        let address_length = match (raw >> 1) & 3 {
            0b00 => AddrLen::Bits32,
            0b10 => AddrLen::Bits64,
            _ => return Err(Error::InvalidArgs),
        };

        let offset = index as u16 * 4 + PciGeneralDeviceCfgOffset::Bar0 as u16;

        location.write32(offset, !0);

        let size_encoded = location.read32(offset);

        if size_encoded == 0 {
            return Err(Error::InvalidArgs);
        }

        #[cfg_attr(target_arch = "loongarch64", expect(unused_variables))]
        let (raw64, size_encoded64) = match address_length {
            AddrLen::Bits32 => (raw as u64, size_encoded as u64 | ((u32::MAX as u64) << 32)),
            AddrLen::Bits64 => {
                let raw64 = raw as u64 | ((location.read32(offset + 4) as u64) << 32);
                location.write32(offset + 4, !0);
                let size_encoded64 =
                    size_encoded as u64 | ((location.read32(offset + 4) as u64) << 32);
                (raw64, size_encoded64)
            }
        };

        let size = decode_size(size_encoded64, BarKind::Memory);

        #[cfg(not(target_arch = "loongarch64"))]
        let base = raw64 & MEMORY_ADDRESS_MASK;
        #[cfg(target_arch = "loongarch64")]
        let base = {
            use core::alloc::Layout;
            crate::arch::alloc_mmio(Layout::from_size_align(size as usize, size as usize).unwrap())
                .unwrap() as u64
        };
        match address_length {
            AddrLen::Bits32 => location.write32(offset, base as u32),
            AddrLen::Bits64 => {
                location.write32(offset, base as u32);
                location.write32(offset + 4, (base >> 32) as u32);
            }
        }

        #[cfg(not(target_arch = "loongarch64"))]
        if base == 0 {
            ostd::info!(
                "presumably uninitialized BAR {} (Memory {:?}, size={}) of PCI device {:?}",
                index,
                address_length,
                size,
                location,
            );
            return Err(Error::InvalidArgs);
        }

        let prefetchable = raw & 0b1000 != 0;

        Ok(MemoryBar {
            base,
            size,
            prefetchable,
            address_length,
            io_memory: Once::new(),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AddrLen {
    Bits32,
    Bits64,
}

#[derive(Clone, Copy, Debug)]
pub struct IoBar {
    base: u32,
    size: u32,
}

impl IoBar {
    pub fn base(&self) -> u32 {
        self.base
    }

    pub fn size(&self) -> u32 {
        self.size
    }

    fn new(location: &PciDeviceLocation, index: u8, raw: u32) -> Result<Self> {
        debug_assert_eq!(raw & 1, 1);

        let offset = index as u16 * 4 + PciGeneralDeviceCfgOffset::Bar0 as u16;

        location.write32(offset, !0);
        let size_encoded = location.read32(offset);

        let size = decode_size(size_encoded as u64, BarKind::Io) as u32;

        let base = raw & IO_ADDRESS_MASK;
        location.write32(offset, base);

        if base == 0 {
            ostd::info!(
                "presumably uninitialized BAR {} (I/O, size={}) of PCI device {:?}",
                index,
                size,
                location,
            );
            return Err(Error::InvalidArgs);
        }

        Ok(Self { base, size })
    }
}

enum BarKind {
    Memory,
    Io,
}

const MEMORY_ADDRESS_MASK: u64 = !0xF;
const IO_ADDRESS_MASK: u32 = !0x3;

fn decode_size(size_encoded: u64, kind: BarKind) -> u64 {
    let mask = match kind {
        BarKind::Memory => MEMORY_ADDRESS_MASK,
        BarKind::Io => IO_ADDRESS_MASK as u64,
    };

    let size_masked = size_encoded & mask;
    size_masked & !(size_masked - 1)
}
