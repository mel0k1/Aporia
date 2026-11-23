use core::ops::RangeInclusive;

use ostd::Error;

use crate::PciDeviceLocation;

pub(crate) fn write32(
    _location: &PciDeviceLocation,
    _offset: u32,
    _value: u32,
) -> Result<(), Error> {
    Err(Error::IoError)
}

pub(crate) fn read32(_location: &PciDeviceLocation, _offset: u32) -> Result<u32, Error> {
    Err(Error::IoError)
}

pub(crate) fn init() -> Option<RangeInclusive<u8>> {
    None
}

pub(crate) const MSIX_DEFAULT_MSG_ADDR: u32 = 0x2400_0000;

pub(crate) fn construct_remappable_msix_address(_remapping_index: u32) -> u32 {
    unimplemented!()
}
