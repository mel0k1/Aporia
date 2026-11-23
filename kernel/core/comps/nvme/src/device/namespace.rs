use aster_block::SECTOR_SIZE;

#[derive(Debug)]
pub(super) struct NvmeNamespace {
    pub id: u32,
    pub nsze: u64,
}

pub(super) const LBA_SIZE: usize = SECTOR_SIZE;
