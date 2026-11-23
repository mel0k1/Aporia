mod block_ptr_tree;
mod indirect_block_manager;

use core::sync::atomic::{AtomicUsize, Ordering};

use aster_block::bio::BioCompleteFn;

use self::block_ptr_tree::ResolvedBlockRange;
pub(super) use self::block_ptr_tree::{BlockPtrTree, RawBlockPtrs};
use super::io_range::{IoRange, IoRangeIter};
use crate::{
    fs::ext2::{fs::Ext2, prelude::*},
    vm::page_cache::PageRun,
};

#[derive(Debug)]
pub(super) struct InodeBlockManager {
    block_ptr_tree: RwMutex<BlockPtrTree>,
    npages: AtomicUsize,
    fs: Weak<Ext2>,
}

impl InodeBlockManager {
    pub(super) fn new(block_ptr_tree: BlockPtrTree, fs: Weak<Ext2>, npages: usize) -> Self {
        Self {
            block_ptr_tree: RwMutex::new(block_ptr_tree),
            npages: AtomicUsize::new(npages),
            fs,
        }
    }

    pub(super) fn fs(&self) -> Result<Arc<Ext2>> {
        self.fs
            .upgrade()
            .ok_or_else(|| errno!(EIO, "filesystem already dropped"))
    }

    pub(super) fn lookup_block(&self, iblock: Iblock) -> Result<Option<Ext2Bid>> {
        let tree = self.block_ptr_tree.read();
        tree.lookup_block(iblock)
    }

    pub(super) fn raw_block_ptrs(&self) -> RawBlockPtrs {
        *self.block_ptr_tree.read().raw_block_ptrs()
    }

    pub(super) fn is_dirty(&self) -> bool {
        self.block_ptr_tree.read().is_dirty()
    }

    pub(super) fn clear_dirty(&self) {
        self.block_ptr_tree.write().clear_dirty();
    }

    pub(super) fn iter_io_ranges(&self, block_range: Range<Iblock>) -> IoRangeIter<'_> {
        let tree = self.block_ptr_tree.read();
        IoRangeIter::new(block_range, tree)
    }

    pub(super) fn truncate_to_byte_len(&self, new_size: usize) {
        let fs = match self.fs() {
            Ok(fs) => fs,
            Err(err) => {
                error!("truncate: failed to get fs reference, err: {:?}", err);
                return;
            }
        };
        let mut tree = self.block_ptr_tree.write();
        tree.truncate_to_byte_len(&fs, new_size)
    }

    pub(super) fn sync_indirect_blocks(&self) -> Result<()> {
        self.block_ptr_tree.write().sync_indirect_blocks()
    }

    pub(super) fn allocate_range_blocks(&self, start_block: usize, end_block: usize) -> Result<()> {
        let fs = self.fs()?;
        let mut tree = self.block_ptr_tree.write();
        let mut current_block = start_block;
        while current_block < end_block {
            let iblock = Iblock::try_from(current_block)
                .map_err(|_| errno!(EINVAL, "logical block number overflow"))?;
            let remaining = u32::try_from(end_block - current_block)
                .map_err(|_| errno!(EINVAL, "block range length overflow"))?;

            let block_range = tree.resolve_block_range(&fs, iblock, remaining)?;
            match block_range {
                ResolvedBlockRange::Existing(range) => {
                    debug_assert!(!range.is_empty());
                    current_block += range.len();
                }
                ResolvedBlockRange::NewlyAllocated(range) => {
                    debug_assert!(!range.is_empty());
                    current_block += range.len();
                }
            }
        }
        Ok(())
    }

    pub(super) fn set_npages(&self, npages: usize) {
        self.npages.store(npages, Ordering::Release);
    }
}

impl BlockAsPageCacheBackend for InodeBlockManager {
    fn submit_read_bio(
        &self,
        idx: usize,
        bio_segment: BioSegment,
        complete_fn: BioCompleteFn,
        io_batch: &mut IoBatch,
    ) -> Result<()> {
        if idx >= self.npages.load(Ordering::Acquire) {
            return Err(errno!(EINVAL, "invalid read size"));
        }
        let iblock = idx as Iblock;
        match self.lookup_block(iblock)? {
            Some(bid) => {
                let fs = self.fs()?;
                fs.read_blocks_async(bid, bio_segment, Some(complete_fn), io_batch)
            }
            None => {
                complete_fn(BioStatus::Zeros);
                Ok(())
            }
        }
    }

    fn submit_read_page_run(&self, mut pages: PageRun<'_>, io_batch: &mut IoBatch) -> Result<()> {
        let start_idx = pages.start_idx();
        let end_idx = start_idx + pages.len();
        if end_idx > self.npages.load(Ordering::Acquire) {
            return Err(errno!(EINVAL, "invalid read size"));
        }

        let iblock_range = start_idx as Iblock..end_idx as Iblock;
        let fs = self.fs()?;
        let mut io_ranges = self.iter_io_ranges(iblock_range);

        while let Some(io_range) = io_ranges.next()? {
            match io_range {
                IoRange::Mapped(device_range) => {
                    let range_len = device_range.len();
                    pages.submit_contiguous_read_pages(
                        fs.block_device(),
                        Bid::new(device_range.start as u64),
                        range_len,
                        io_batch,
                    )?;
                }
                IoRange::Hole(hole_range) => {
                    let hole_len = hole_range.len();
                    for locked_page in pages.by_ref().take(hole_len) {
                        locked_page.fill_zeros(0, PAGE_SIZE).unwrap();
                        locked_page.set_up_to_date();
                    }
                }
            }
        }

        Ok(())
    }

    fn submit_write_bio(
        &self,
        idx: usize,
        bio_segment: BioSegment,
        complete_fn: BioCompleteFn,
        io_batch: &mut IoBatch,
    ) -> Result<()> {
        if idx >= self.npages.load(Ordering::Acquire) {
            return Err(errno!(EINVAL, "invalid write size"));
        }
        let iblock = idx as Iblock;
        let fs = self.fs()?;

        if let Some(bid) = self.lookup_block(iblock)? {
            return fs.write_blocks_async(bid, bio_segment, Some(complete_fn), io_batch);
        }

        let mut tree = self.block_ptr_tree.write();
        let step = tree.resolve_block_range(&fs, iblock, bio_segment.nblocks() as u32)?;
        let bid = match step {
            ResolvedBlockRange::NewlyAllocated(r) => r.start,
            ResolvedBlockRange::Existing(r) => r.start,
        };

        fs.write_blocks_async(bid, bio_segment, Some(complete_fn), io_batch)
    }

    fn submit_write_page_run(&self, mut pages: PageRun<'_>, io_batch: &mut IoBatch) -> Result<()> {
        let start_idx = pages.start_idx();
        let end_idx = start_idx + pages.len();
        if end_idx > self.npages.load(Ordering::Acquire) {
            return Err(errno!(EINVAL, "invalid write size"));
        }
        self.allocate_range_blocks(start_idx, end_idx)?;
        let iblock_range = start_idx as Iblock..end_idx as Iblock;
        let fs = self.fs()?;
        let mut io_ranges = self.iter_io_ranges(iblock_range);

        while let Some(io_range) = io_ranges.next()? {
            match io_range {
                IoRange::Mapped(device_range) => {
                    let range_len = device_range.len();
                    pages.submit_contiguous_write_pages(
                        fs.block_device(),
                        Bid::new(device_range.start as u64),
                        range_len,
                        io_batch,
                    )?;
                }
                IoRange::Hole(_) => {
                    return Err(errno!(EIO, "unexpected hole after writeback allocation"));
                }
            }
        }

        Ok(())
    }
}
