macro_rules! __log_prefix {
    () => {
        "iommu: "
    };
}

mod dma_remapping;
mod fault;
mod interrupt_remapping;
mod invalidate;
mod registers;

pub(crate) use dma_remapping::{IommuPtConfig, has_dma_remapping, map, unmap};
pub(in crate::arch) use interrupt_remapping::{
    IrtEntryHandle, alloc_irt_entry, has_interrupt_remapping,
};

use crate::{io::IoMemAllocatorBuilder, mm::page_table::PageTableError};

#[derive(Debug)]
pub(crate) enum IommuError {
    NoIommu,
    #[expect(dead_code)]
    ModificationError(PageTableError),
}

pub(crate) fn init(io_mem_builder: &IoMemAllocatorBuilder) -> Result<(), IommuError> {
    registers::init(io_mem_builder)?;
    invalidate::init();
    dma_remapping::init();
    interrupt_remapping::init();
    Ok(())
}
