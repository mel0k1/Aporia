use alloc::vec::Vec;

use align_ext::AlignExt;
use int_to_c_enum::TryFromInt;

use self::{
    msix::RawCapabilityMsix,
    vendor::{CapabilityVndrData, RawCapabilityVndr},
};
use crate::{
    cfg_space::{PciGeneralDeviceCfgOffset, Status},
    common_device::PciCommonDevice,
};

pub mod msix;
pub mod vendor;

#[derive(Debug, Default)]
pub(super) struct RawCapabilities {
    msix: Option<RawCapabilityMsix>,
    vndr: Vec<RawCapabilityVndr>,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, TryFromInt)]
enum CapabilityType {
    Pm = 0x01,
    Agp = 0x02,
    Vpd = 0x03,
    SlotId = 0x04,
    Msi = 0x05,
    Chswp = 0x06,
    PciX = 0x07,
    Hp = 0x08,
    Vndr = 0x09,
    Dbg = 0x0A,
    Ccrc = 0x0B,
    Shpc = 0x0C,
    Ssvid = 0x0D,
    Agp3 = 0x0E,
    Secdev = 0x0F,
    Exp = 0x10,
    Msix = 0x11,
    Sata = 0x12,
    Af = 0x13,
    Ea = 0x14,
}

impl RawCapabilities {
    const CAPABILITY_TOP: u16 = 0xFC;

    pub(super) fn parse(dev: &PciCommonDevice) -> Self {
        if !dev.read_status().contains(Status::CAPABILITIES_LIST) {
            return Self::default();
        }

        const CAP_OFFSET: u16 = PciGeneralDeviceCfgOffset::CapabilitiesPointer as u16;
        let mut cap_ptr =
            (dev.location().read8(CAP_OFFSET) as u16).align_down(align_of::<u32>() as _);
        let mut cap_ptr_vec = Vec::new();

        while cap_ptr > 0 {
            cap_ptr_vec.push(cap_ptr);
            cap_ptr = (dev.location().read8(cap_ptr + 1) as u16).align_down(align_of::<u32>() as _);
        }
        cap_ptr_vec.sort();

        cap_ptr_vec.push(Self::CAPABILITY_TOP);

        let mut caps = Self::default();

        let length = cap_ptr_vec.len();
        for i in 0..length - 1 {
            let cap_ptr = cap_ptr_vec[i];
            let next_ptr = cap_ptr_vec[i + 1];
            let raw_cap_type = dev.location().read8(cap_ptr);

            let Ok(cap_type) = CapabilityType::try_from(raw_cap_type) else {
                continue;
            };
            match cap_type {
                CapabilityType::Msix => {
                    if caps.msix.is_some() {
                        ostd::warn!(
                            "superfluous MSI-X Capability structures at {:?} are ignored",
                            dev.location()
                        );
                        continue;
                    }
                    caps.msix = Some(RawCapabilityMsix::parse(dev, cap_ptr));
                }
                CapabilityType::Vndr => {
                    caps.vndr
                        .push(RawCapabilityVndr::new(cap_ptr, next_ptr - cap_ptr));
                }
                _ => {}
            }
        }

        caps
    }

    pub(super) fn msix(&self) -> Option<&RawCapabilityMsix> {
        self.msix.as_ref()
    }

    pub(super) fn iter_vndr_data<'a>(
        &'a self,
        device: &'a PciCommonDevice,
    ) -> impl Iterator<Item = CapabilityVndrData<'a>> {
        self.vndr
            .iter()
            .map(move |raw_vndr| CapabilityVndrData::new(device, raw_vndr))
    }
}
