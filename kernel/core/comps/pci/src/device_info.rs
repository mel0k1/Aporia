use crate::cfg_space::PciCommonCfgOffset;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PciDeviceLocation {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
}

impl PciDeviceLocation {
    pub const MIN_DEVICE: u8 = 0;
    pub const MAX_DEVICE: u8 = 31;

    pub const MIN_FUNCTION: u8 = 0;
    pub const MAX_FUNCTION: u8 = 7;
}

impl PciDeviceLocation {
    pub fn read8(&self, offset: u16) -> u8 {
        let val = self.read32(offset & !0b11);
        ((val >> ((offset as usize & 0b11) << 3)) & 0xFF) as u8
    }

    pub fn read16(&self, offset: u16) -> u16 {
        debug_assert!(
            (offset & 0b1) == 0,
            "misaligned PCI configuration dword u16 read"
        );

        let val = self.read32(offset & !0b11);
        ((val >> ((offset as usize & 0b10) << 3)) & 0xFFFF) as u16
    }

    pub fn read32(&self, offset: u16) -> u32 {
        debug_assert!(
            (offset & 0b11) == 0,
            "misaligned PCI configuration dword u32 read"
        );

        crate::arch::read32(self, offset as u32).unwrap()
    }

    pub fn write8(&self, offset: u16, val: u8) {
        let old = self.read32(offset & !0b11);
        let dest = (offset as usize & 0b11) << 3;
        let mask = (0xFF << dest) as u32;
        self.write32(
            offset & !0b11,
            (((val as u32) << dest) | (old & !mask)).to_le(),
        );
    }

    pub fn write16(&self, offset: u16, val: u16) {
        debug_assert!(
            (offset & 0b1) == 0,
            "misaligned PCI configuration dword u16 write"
        );

        let old = self.read32(offset & !0b11);
        let dest = (offset as usize & 0b10) << 3;
        let mask = (0xFFFF << dest) as u32;
        self.write32(
            offset & !0b11,
            (((val as u32) << dest) | (old & !mask)).to_le(),
        );
    }

    pub fn write32(&self, offset: u16, val: u32) {
        debug_assert!(
            (offset & 0b11) == 0,
            "misaligned PCI configuration dword u32 write"
        );

        crate::arch::write32(self, offset as u32, val).unwrap()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PciDeviceId {
    pub vendor_id: u16,
    pub device_id: u16,
    pub revision_id: u8,
    pub prog_if: u8,
    pub subclass: u8,
    pub class: u8,
}

impl PciDeviceId {
    pub(super) fn new(location: PciDeviceLocation) -> Self {
        let vendor_id = location.read16(PciCommonCfgOffset::VendorId as u16);
        let device_id = location.read16(PciCommonCfgOffset::DeviceId as u16);
        let revision_id = location.read8(PciCommonCfgOffset::RevisionId as u16);
        let prog_if = location.read8(PciCommonCfgOffset::ProgIf as u16);
        let subclass = location.read8(PciCommonCfgOffset::SubclassCode as u16);
        let class = location.read8(PciCommonCfgOffset::BaseClassCode as u16);
        Self {
            vendor_id,
            device_id,
            revision_id,
            prog_if,
            subclass,
            class,
        }
    }
}
