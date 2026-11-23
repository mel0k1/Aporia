use crate::{
    capability::{RawCapabilities, msix::RawCapabilityMsix, vendor::CapabilityVndrData},
    cfg_space::{AddrLen, Bar, Command, PciBridgeCfgOffset, PciCommonCfgOffset, Status},
    device_info::{PciDeviceId, PciDeviceLocation},
};

#[derive(Debug)]
pub struct PciCommonDevice {
    device_id: PciDeviceId,
    location: PciDeviceLocation,
    header_type: PciHeaderType,
    bar_manager: BarManager,
    capabilities: RawCapabilities,
}

impl PciCommonDevice {
    pub fn device_id(&self) -> &PciDeviceId {
        &self.device_id
    }

    pub fn location(&self) -> &PciDeviceLocation {
        &self.location
    }

    pub fn bar_manager(&self) -> &BarManager {
        &self.bar_manager
    }

    pub fn device_type(&self) -> PciDeviceType {
        self.header_type.device_type()
    }

    pub fn has_multi_funcs(&self) -> bool {
        self.header_type.has_multi_funcs()
    }

    pub fn read_command(&self) -> Command {
        Command::from_bits_truncate(self.location.read16(PciCommonCfgOffset::Command as u16))
    }

    pub fn write_command(&self, command: Command) {
        self.location
            .write16(PciCommonCfgOffset::Command as u16, command.bits())
    }

    pub fn read_status(&self) -> Status {
        Status::from_bits_truncate(self.location.read16(PciCommonCfgOffset::Status as u16))
    }

    pub(crate) fn raw_msix_capability(&self) -> Option<&RawCapabilityMsix> {
        self.capabilities.msix()
    }

    pub fn iter_vndr_capability(&self) -> impl Iterator<Item = CapabilityVndrData<'_>> {
        self.capabilities.iter_vndr_data(self)
    }

    pub(super) fn new(location: PciDeviceLocation) -> Option<Self> {
        if location.read16(0) == 0xFFFF {
            return None;
        }

        let device_id = PciDeviceId::new(location);

        let mut header_type =
            PciHeaderType::try_from_raw(location.read8(PciCommonCfgOffset::HeaderType as u16))?;
        if let PciDeviceType::PciToPciBridge(primary_bus, secondary_bus, subordinate_bus) =
            &mut header_type.device_type
        {
            *primary_bus = location.read8(PciBridgeCfgOffset::PrimaryBusNumber as u16);
            *secondary_bus = location.read8(PciBridgeCfgOffset::SecondaryBusNumber as u16);
            *subordinate_bus = location.read8(PciBridgeCfgOffset::SubordinateBusNumber as u16);
        }

        let bar_manager = BarManager {
            bars: [const { None }; 6],
        };
        let capabilities = RawCapabilities::default();

        let mut device = Self {
            device_id,
            location,
            header_type,
            bar_manager,
            capabilities,
        };

        let command_val = device.read_command() | Command::BUS_MASTER;
        device.write_command(command_val - (Command::MEMORY_SPACE | Command::IO_SPACE));
        device.bar_manager = BarManager::new(device.header_type.device_type(), location);
        device.write_command(command_val | (Command::MEMORY_SPACE | Command::IO_SPACE));

        device.capabilities = RawCapabilities::parse(&device);

        Some(device)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PciHeaderType {
    device_type: PciDeviceType,
    has_multi_funcs: bool,
}

impl PciHeaderType {
    fn try_from_raw(raw: u8) -> Option<Self> {
        let device_type = PciDeviceType::try_from_raw(raw & 0x7F)?;
        let has_multi_funcs = (raw & 0x80) != 0;

        Some(Self {
            device_type,
            has_multi_funcs,
        })
    }

    fn device_type(self) -> PciDeviceType {
        self.device_type
    }

    fn has_multi_funcs(self) -> bool {
        self.has_multi_funcs
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PciDeviceType {
    GeneralDevice,
    PciToPciBridge(u8, u8, u8),
    PciToCardbusBridge,
}

impl PciDeviceType {
    pub fn try_from_raw(raw: u8) -> Option<Self> {
        match raw {
            0x00 => Some(PciDeviceType::GeneralDevice),
            0x01 => Some(PciDeviceType::PciToPciBridge(0, 0, 0)),
            0x02 => Some(PciDeviceType::PciToCardbusBridge),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub struct BarManager {
    bars: [Option<Bar>; 6],
}

impl BarManager {
    pub fn bar(&self, idx: u8) -> Option<&Bar> {
        self.bars[idx as usize].as_ref()
    }

    fn new(device_type: PciDeviceType, location: PciDeviceLocation) -> Self {
        let mut bars = [None, None, None, None, None, None];

        let max = match device_type {
            PciDeviceType::GeneralDevice => 6,
            PciDeviceType::PciToPciBridge(_, _, _) => 2,
            PciDeviceType::PciToCardbusBridge => 0,
        };
        let mut idx = 0;
        while idx < max {
            let mut idx_step = 1;
            let Ok(bar) = Bar::new(location, idx) else {
                idx += idx_step;
                continue;
            };

            if let Bar::Memory(memory_bar) = &bar
                && memory_bar.address_length() == AddrLen::Bits64
            {
                idx_step += 1;
            }

            bars[idx as usize] = Some(bar);
            idx += idx_step;
        }

        Self { bars }
    }
}
