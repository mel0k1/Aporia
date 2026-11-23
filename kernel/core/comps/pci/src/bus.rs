use alloc::sync::Arc;

use aster_device::{
    bus::{Bus, BusDevice},
    common::Attr,
    uevent::UeventVars,
};
use ostd::Result;

use crate::{capability::msix::CapabilityMsixData, common_device::PciCommonDevice};

pub struct PciBus;

pub type PciDevice = BusDevice<PciBus>;

pub trait PciDeviceExt {
    fn acquire_msix_capability(self: &Arc<Self>) -> Result<Option<CapabilityMsixData>>;
}

impl PciDeviceExt for PciDevice {
    fn acquire_msix_capability(self: &Arc<Self>) -> Result<Option<CapabilityMsixData>> {
        CapabilityMsixData::new(self)
    }
}

#[derive(Clone, Copy, Debug)]
pub enum PciMatch {
    Device {
        vendor: u16,
        first: u16,
        last: u16,
    },
    Class {
        class: u8,
        subclass: u8,
        prog_if: u8,
    },
}

impl Bus for PciBus {
    const NAME: &'static str = "pci";

    type Device = PciCommonDevice;
    type MatchData = [PciMatch];

    fn matches(&self, dev: &PciCommonDevice, rules: &Self::MatchData) -> bool {
        let id = dev.device_id();
        rules.iter().any(|rule| match *rule {
            PciMatch::Device {
                vendor,
                first,
                last,
            } => id.vendor_id == vendor && (first..=last).contains(&id.device_id),
            PciMatch::Class {
                class,
                subclass,
                prog_if,
            } => id.class == class && id.subclass == subclass && id.prog_if == prog_if,
        })
    }

    fn dev_attrs(&self) -> &'static [Attr<PciDevice>] {
        const ATTRS: &[Attr<PciDevice>] = &[
            Attr::ro("vendor", |dev, w| {
                writeln!(w, "0x{:04x}", dev.device_id().vendor_id)?;
                Ok(())
            }),
            Attr::ro("device", |dev, w| {
                writeln!(w, "0x{:04x}", dev.device_id().device_id)?;
                Ok(())
            }),
            Attr::ro("class", |dev, w| {
                let id = dev.device_id();
                writeln!(w, "0x{:02x}{:02x}{:02x}", id.class, id.subclass, id.prog_if)?;
                Ok(())
            }),
            Attr::ro("revision", |dev, w| {
                writeln!(w, "0x{:02x}", dev.device_id().revision_id)?;
                Ok(())
            }),
        ];
        ATTRS
    }

    fn append_uevent_vars(
        &self,
        dev: &PciDevice,
        vars: &mut UeventVars,
    ) -> aster_device::common::Result<()> {
        let id = dev.device_id();
        let class =
            (u32::from(id.class) << 16) | (u32::from(id.subclass) << 8) | u32::from(id.prog_if);
        let location = dev.location();

        vars.add("PCI_CLASS", format_args!("{class:04X}"))?;
        vars.add(
            "PCI_ID",
            format_args!("{:04X}:{:04X}", id.vendor_id, id.device_id),
        )?;
        vars.add(
            "PCI_SLOT_NAME",
            format_args!(
                "0000:{:02x}:{:02x}.{}",
                location.bus, location.device, location.function
            ),
        )?;

        Ok(())
    }
}
