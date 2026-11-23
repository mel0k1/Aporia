use crate::mm::{Daddr, Paddr};

#[derive(Debug)]
pub(crate) enum IommuError {
    NoIommu,
}

pub(crate) unsafe fn map(_daddr: Daddr, _paddr: Paddr) -> Result<(), IommuError> {
    Err(IommuError::NoIommu)
}

pub(crate) fn unmap(_daddr: Daddr) -> Result<(), IommuError> {
    Err(IommuError::NoIommu)
}

pub(crate) fn init() -> Result<(), IommuError> {
    Err(IommuError::NoIommu)
}

pub(crate) fn has_dma_remapping() -> bool {
    false
}

pub(crate) fn has_interrupt_remapping() -> bool {
    false
}
