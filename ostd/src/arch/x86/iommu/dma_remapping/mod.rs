pub(super) use context_table::RootTable;
pub(crate) use second_stage::IommuPtConfig;
use spin::Once;

use super::IommuError;
use crate::{
    arch::iommu::registers::{CapabilitySagaw, IOMMU_REGS},
    info,
    mm::{Daddr, PageTable},
    prelude::Paddr,
    sync::{LocalIrqDisabled, SpinLock},
    warn,
};

mod context_table;
mod second_stage;

pub(crate) fn has_dma_remapping() -> bool {
    PAGE_TABLE.get().is_some()
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct PciDeviceLocation {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
}

impl PciDeviceLocation {
    const MIN_BUS: u8 = 0;
    const MAX_BUS: u8 = 255;

    const MIN_DEVICE: u8 = 0;
    const MAX_DEVICE: u8 = 31;

    const MIN_FUNCTION: u8 = 0;
    const MAX_FUNCTION: u8 = 7;

    fn all() -> impl Iterator<Item = PciDeviceLocation> {
        let all_bus = Self::MIN_BUS..=Self::MAX_BUS;
        let all_dev = Self::MIN_DEVICE..=Self::MAX_DEVICE;
        let all_func = Self::MIN_FUNCTION..=Self::MAX_FUNCTION;

        all_bus
            .flat_map(move |bus| all_dev.clone().map(move |dev| (bus, dev)))
            .flat_map(move |(bus, dev)| all_func.clone().map(move |func| (bus, dev, func)))
            .map(|(bus, dev, func)| PciDeviceLocation {
                bus,
                device: dev,
                function: func,
            })
    }

    fn zero() -> Self {
        Self {
            bus: 0,
            device: 0,
            function: 0,
        }
    }
}

pub(crate) unsafe fn map(daddr: Daddr, paddr: Paddr) -> Result<(), IommuError> {
    let Some(table) = PAGE_TABLE.get() else {
        return Err(IommuError::NoIommu);
    };

    let mut locked_table = table.lock();
    let res = unsafe { locked_table.map(PciDeviceLocation::zero(), daddr, paddr) };

    match res {
        Ok(()) => Ok(()),
        Err(context_table::ContextTableError::InvalidDeviceId) => unreachable!(),
        Err(context_table::ContextTableError::ModificationError(err)) => {
            Err(IommuError::ModificationError(err))
        }
    }
}

pub(crate) fn unmap(daddr: Daddr) -> Result<(), IommuError> {
    let Some(table) = PAGE_TABLE.get() else {
        return Err(IommuError::NoIommu);
    };

    let mut locked_table = table.lock();
    let res = locked_table.unmap(PciDeviceLocation::zero(), daddr);

    match res {
        Ok(()) => Ok(()),
        Err(context_table::ContextTableError::InvalidDeviceId) => unreachable!(),
        Err(context_table::ContextTableError::ModificationError(err)) => {
            Err(IommuError::ModificationError(err))
        }
    }
}

pub(super) fn init() {
    if !IOMMU_REGS
        .get()
        .unwrap()
        .lock()
        .read_capability()
        .supported_adjusted_guest_address_widths()
        .contains(CapabilitySagaw::AGAW_39BIT_3LP)
    {
        warn!("3-level page tables not supported, disabling DMA remapping");
        return;
    }

    let mut root_table = RootTable::new();
    let page_table = PageTable::<IommuPtConfig>::empty();
    for table in PciDeviceLocation::all() {
        root_table.specify_device_page_table(table, unsafe { page_table.shallow_copy() })
    }
    PAGE_TABLE.call_once(|| SpinLock::new(root_table));

    let mut iommu_regs = IOMMU_REGS.get().unwrap().lock();
    iommu_regs.enable_dma_remapping(PAGE_TABLE.get().unwrap());
    info!("DMA remapping enabled");
}

static PAGE_TABLE: Once<SpinLock<RootTable, LocalIrqDisabled>> = Once::new();
