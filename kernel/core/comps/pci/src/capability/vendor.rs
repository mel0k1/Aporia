use ostd::{Error, Result};

use crate::common_device::PciCommonDevice;

#[derive(Debug)]
pub(super) struct RawCapabilityVndr {
    cap_ptr: u16,
    length: u16,
}

impl RawCapabilityVndr {
    pub(super) fn new(cap_ptr: u16, length: u16) -> Self {
        Self { cap_ptr, length }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CapabilityVndrData<'a> {
    device: &'a PciCommonDevice,
    cap_ptr: u16,
    length: u16,
}

impl<'a> CapabilityVndrData<'a> {
    pub(super) fn new(device: &'a PciCommonDevice, raw_cap: &RawCapabilityVndr) -> Self {
        Self {
            device,
            cap_ptr: raw_cap.cap_ptr,
            length: raw_cap.length,
        }
    }

    #[expect(clippy::len_without_is_empty)]
    pub fn len(&self) -> u16 {
        self.length
    }

    pub fn read8(&self, offset: u16) -> Result<u8> {
        self.check_range(offset, size_of::<u8>() as u16)?;
        Ok(self.device.location().read8(self.cap_ptr + offset))
    }

    pub fn write8(&self, offset: u16, value: u8) -> Result<()> {
        self.check_range(offset, size_of::<u8>() as u16)?;
        self.device.location().write8(self.cap_ptr + offset, value);
        Ok(())
    }

    pub fn read16(&self, offset: u16) -> Result<u16> {
        self.check_range(offset, size_of::<u16>() as u16)?;
        Ok(self.device.location().read16(self.cap_ptr + offset))
    }

    pub fn write16(&self, offset: u16, value: u16) -> Result<()> {
        self.check_range(offset, size_of::<u16>() as u16)?;
        self.device.location().write16(self.cap_ptr + offset, value);
        Ok(())
    }

    pub fn read32(&self, offset: u16) -> Result<u32> {
        self.check_range(offset, size_of::<u32>() as u16)?;
        Ok(self.device.location().read32(self.cap_ptr + offset))
    }

    pub fn write32(&self, offset: u16, value: u32) -> Result<()> {
        self.check_range(offset, size_of::<u32>() as u16)?;
        self.device.location().write32(self.cap_ptr + offset, value);
        Ok(())
    }

    fn check_range(&self, offset: u16, size: u16) -> Result<()> {
        if let Some(end) = offset.checked_add(size)
            && end <= self.length
        {
            return Ok(());
        }
        Err(Error::InvalidArgs)
    }
}
