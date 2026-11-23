use core::fmt::Debug;

use bitflags::bitflags;

pub(in crate::arch::iommu) struct ExtendedCapability(u64);

impl ExtendedCapability {
    pub(super) const fn new(value: u64) -> Self {
        Self(value)
    }

    pub(in crate::arch::iommu) const fn flags(&self) -> ExtendedCapabilityFlags {
        ExtendedCapabilityFlags::from_bits_truncate(self.0)
    }

    pub(super) const fn iotlb_register_offset(&self) -> u64 {
        const IRO_MASK: u64 = 0x3FF << 8;
        (self.0 & IRO_MASK) >> 8
    }

    const fn maximum_handle_mask(&self) -> u64 {
        const MHMV_MASK: u64 = 0xF << 20;
        (self.0 & MHMV_MASK) >> 20
    }

    const fn pasid_size(&self) -> u64 {
        const PSS_MASK: u64 = 0x1F << 35;
        (self.0 & PSS_MASK) >> 35
    }
}

impl Debug for ExtendedCapability {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ExtendedCapability")
            .field("flags", &self.flags())
            .field("maximum_handle_mask", &self.maximum_handle_mask())
            .field("pasid_size", &self.pasid_size())
            .field("iotlb_register_offset", &self.iotlb_register_offset())
            .field("raw", &self.0)
            .finish()
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub struct ExtendedCapabilityFlags: u64 {
        const C      = 1 << 0;
        const QI     = 1 << 1;
        const DT     = 1 << 2;
        const IR     = 1 << 3;
        const EIM    = 1 << 4;
        const PT     = 1 << 6;
        const SC     = 1 << 7;
        const MTS    = 1 << 25;
        const NEST   = 1 << 26;
        const PRS    = 1 << 29;
        const ERS    = 1 << 30;
        const SRS    = 1 << 31;
        const NWFS   = 1 << 33;
        const EAFS   = 1 << 34;
        const PASID  = 1 << 40;
        const DIT    = 1 << 41;
        const PDS    = 1 << 42;
        const SMTS   = 1 << 43;
        const VCS    = 1 << 44;
        const SSADS  = 1 << 45;
        const SSTS   = 1 << 46;
        const FSTS   = 1 << 47;
        const SMPWCS = 1 << 48;
        const RPS    = 1 << 49;
        const PMS    = 1 << 51;
        const ADMS   = 1 << 52;
        const RPRIVS = 1 << 53;
        const SMS    = 1 << 58;
    }
}
