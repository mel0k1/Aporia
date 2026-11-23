use core::fmt::Debug;

use bitflags::bitflags;

pub(in crate::arch::iommu) struct Capability(u64);

impl Capability {
    pub(in crate::arch::iommu) const fn new(value: u64) -> Self {
        Self(value)
    }

    const fn flags(&self) -> CapabilityFlags {
        CapabilityFlags::from_bits_truncate(self.0)
    }

    pub(in crate::arch::iommu) const fn fault_recording_number(&self) -> u64 {
        const NFR_MASK: u64 = 0xFF << 40;
        (self.0 & NFR_MASK) >> 40
    }

    const fn maximum_address_mask_value(&self) -> u64 {
        const MAMV_MASK: u64 = 0x3F << 48;
        (self.0 & MAMV_MASK) >> 48
    }

    const fn domain_support_number(&self) -> u64 {
        const ND_MASK: u64 = 0x7;
        self.0 & ND_MASK
    }

    pub(in crate::arch::iommu) const fn supported_adjusted_guest_address_widths(
        &self,
    ) -> CapabilitySagaw {
        CapabilitySagaw::from_bits_truncate(self.0 >> 8)
    }

    pub(in crate::arch::iommu) const fn fault_recording_register_offset(&self) -> u64 {
        const FRO_MASK: u64 = 0x3FF << 24;
        (self.0 & FRO_MASK) >> 24
    }

    const fn second_stage_large_page_support(&self) -> CapabilitySslps {
        CapabilitySslps::from_bits_truncate(self.0 >> 34)
    }

    const fn maximum_guest_address_width(&self) -> u64 {
        const MGAW_MASK: u64 = 0x3F << 16;
        (self.0 & MGAW_MASK) >> 16
    }
}

impl Debug for Capability {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Capability")
            .field("flags", &self.flags())
            .field(
                "maximum_guest_address_width",
                &self.maximum_guest_address_width(),
            )
            .field(
                "second_stage_large_page_support",
                &self.second_stage_large_page_support(),
            )
            .field(
                "fault_recording_register_offset",
                &self.fault_recording_register_offset(),
            )
            .field(
                "supported_adjusted_guest_address_widths",
                &self.supported_adjusted_guest_address_widths(),
            )
            .field("domain_support_number", &self.domain_support_number())
            .field(
                "maximum_address_mask_value",
                &self.maximum_address_mask_value(),
            )
            .field("fault_recording_number", &self.fault_recording_number())
            .field("raw", &self.0)
            .finish()
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub struct CapabilityFlags: u64 {
        const RWBF    = 1 << 4;
        const PLMR    = 1 << 5;
        const PHMR    = 1 << 6;
        const CM      = 1 << 7;
        const ZLR     = 1 << 22;
        const PSI     = 1 << 39;
        const DWD     = 1 << 54;
        const DRD     = 1 << 55;
        const FS1GP   = 1 << 56;
        const PI      = 1 << 59;
        const FS5LP   = 1 << 60;
        const ECMDS   = 1 << 61;
        const ESIRTPS = 1 << 62;
        const ESRTPS  = 1 << 63;
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub struct CapabilitySagaw: u64 {
        const AGAW_39BIT_3LP = 1 << 1;
        const AGAW_48BIT_4LP = 1 << 2;
        const AGAW_57BIT_5LP = 1 << 3;
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub struct CapabilitySslps: u64 {
        const PAGE_21BIT_2MB = 1 << 0;
        const PAGE_30BIT_1GB = 1 << 1;
    }
}
