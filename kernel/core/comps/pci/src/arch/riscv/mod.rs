//! PCI bus access

use core::ops::RangeInclusive;

use fdt_util::AcquireIoMems;
use ostd::{Error, arch::boot::DEVICE_TREE, io::IoMem, mm::VmIoOnce, warn};
use spin::Once;

use crate::PciDeviceLocation;

static PCI_ECAM_CFG_SPACE: Once<IoMem> = Once::new();

pub(crate) fn write32(location: &PciDeviceLocation, offset: u32, value: u32) -> Result<(), Error> {
    if offset > PCI_ECAM_MAX_OFFSET {
        return Err(Error::InvalidArgs);
    }
    PCI_ECAM_CFG_SPACE.get().ok_or(Error::IoError)?.write_once(
        (encode_as_address_offset(location) | offset) as usize,
        &value,
    )
}

pub(crate) fn read32(location: &PciDeviceLocation, offset: u32) -> Result<u32, Error> {
    if offset > PCI_ECAM_MAX_OFFSET {
        return Err(Error::InvalidArgs);
    }
    PCI_ECAM_CFG_SPACE
        .get()
        .ok_or(Error::IoError)?
        .read_once((encode_as_address_offset(location) | offset) as usize)
}

/// The maximum offset in the 12-bit configuration space when using [`encode_as_address_offset`].
const PCI_ECAM_MAX_OFFSET: u32 = 0xffc;

const PCI_ECAM_BUS_SHIFT: u32 = 20;
const PCI_ECAM_DEVICE_SHIFT: u32 = 15;
const PCI_ECAM_FUNCTION_SHIFT: u32 = 12;

/// Encodes the bus, device, and function into an address offset in the PCI MMIO region.
fn encode_as_address_offset(location: &PciDeviceLocation) -> u32 {
    ((location.bus as u32) << PCI_ECAM_BUS_SHIFT)
        | ((location.device as u32) << PCI_ECAM_DEVICE_SHIFT)
        | ((location.function as u32) << PCI_ECAM_FUNCTION_SHIFT)
}

/// Initializes the platform-specific module for accessing the PCI configuration space.
///
/// Returns a range for the PCI bus number, or [`None`] if there is no PCI bus.
pub(crate) fn init() -> Option<RangeInclusive<u8>> {
    let Some(pci) = DEVICE_TREE
        .get()
        .unwrap()
        .find_compatible(&["pci-host-ecam-generic"])
    else {
        warn!("no generic host controller node found in the device tree");
        return None;
    };

    let bus_range = if let Some(prop) = pci.property("bus-range") {
        if prop.value.len() != 8 || prop.value[0..3] != [0, 0, 0] || prop.value[4..7] != [0, 0, 0] {
            warn!(
                "node should have a `bus-range` property with two bytes, but found `{:?}`",
                prop.value
            );
            return None;
        }
        if prop.value[3] != 0 {
            warn!(
                "node with a non-zero bus start `{}` is not supported yet",
                prop.value[3]
            );
            return None;
        }
        prop.value[3]..=prop.value[7]
    } else {
        0..=255
    };

    let io_size = (*bus_range.end() as usize + 1) << PCI_ECAM_BUS_SHIFT;
    let [io_mem] = pci.acquire_io_mems([io_size])?;

    PCI_ECAM_CFG_SPACE.call_once(|| io_mem);

    Some(bus_range)
}

pub(crate) const MSIX_DEFAULT_MSG_ADDR: u32 = 0x2400_0000;

pub(crate) fn construct_remappable_msix_address(_remapping_index: u32) -> u32 {
    unimplemented!()
}
