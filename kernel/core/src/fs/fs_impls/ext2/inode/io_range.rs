use super::block_manager::BlockPtrTree;
use crate::fs::ext2::prelude::*;

#[derive(Debug, Eq, PartialEq)]
pub(super) enum IoRange {
    Mapped(Range<Ext2Bid>),
    Hole(Range<Iblock>),
}

pub(super) struct IoRangeIter<'a> {
    range: Range<Iblock>,
    block_ptr_tree: RwMutexReadGuard<'a, BlockPtrTree>,
}

impl<'a> IoRangeIter<'a> {
    pub(super) fn new(
        range: Range<Iblock>,
        block_ptr_tree: RwMutexReadGuard<'a, BlockPtrTree>,
    ) -> Self {
        Self {
            range,
            block_ptr_tree,
        }
    }

    pub(super) fn next(&mut self) -> Result<Option<IoRange>> {
        if self.range.is_empty() {
            return Ok(None);
        }

        let start_iblock = self.range.start;
        let max_blocks = self.range.len() as u32;
        let device_block_range = self
            .block_ptr_tree
            .lookup_block_range(start_iblock, max_blocks)?;

        if device_block_range.is_empty() {
            let hole_len = self
                .block_ptr_tree
                .approx_hole_blocks(start_iblock, max_blocks)?;
            debug_assert!(hole_len > 0);
            self.range.start += hole_len;
            return Ok(Some(IoRange::Hole(start_iblock..start_iblock + hole_len)));
        }

        self.range.start += device_block_range.len() as u32;
        Ok(Some(IoRange::Mapped(device_block_range)))
    }
}

#[cfg(ktest)]
mod test {
    use ostd::prelude::ktest;

    use super::*;
    use crate::{
        fs::fs_impls::ext2::{
            inode::{RAW_BLOCK_PTRS_LEN, block_manager::RawBlockPtrs},
            test_utils::Ext2FixtureBuilder,
        },
        time::clocks,
    };

    fn make_block_ptr_tree(
        block_ptrs: [u32; RAW_BLOCK_PTRS_LEN],
        sector_count: u32,
        fs: &Arc<crate::fs::fs_impls::ext2::fs::Ext2>,
    ) -> BlockPtrTree {
        BlockPtrTree::new(
            RawBlockPtrs::new(sector_count, block_ptrs),
            Arc::downgrade(fs),
        )
    }

    #[ktest]
    fn io_range_iter_yields_mapped_and_holes() {
        clocks::init_for_ktest();
        let f = Ext2FixtureBuilder::new(2, 256).build().unwrap();

        let mut block_ptrs = [0u32; RAW_BLOCK_PTRS_LEN];
        block_ptrs[0] = 50;
        block_ptrs[1] = 51;
        block_ptrs[2] = 52;
        block_ptrs[7] = 60;
        block_ptrs[8] = 61;

        let tree = make_block_ptr_tree(block_ptrs, 0, &f.ext2);
        let lock = RwMutex::new(tree);
        let guard = lock.read();

        let mut iter = IoRangeIter::new(0..9, guard);

        let item = iter.next().unwrap().unwrap();
        assert_eq!(item, IoRange::Mapped(50..53));

        let item = iter.next().unwrap().unwrap();
        assert_eq!(item, IoRange::Hole(3..7));

        let item = iter.next().unwrap().unwrap();
        assert_eq!(item, IoRange::Mapped(60..62));

        let item = iter.next().unwrap();
        assert!(item.is_none());
    }
}
