#![short_vis_path::add(ext2)]

use super::{super::Ext2, Inode, InodeInner, RawInode};
use crate::fs::ext2::{prelude::*, utils};

impl Inode {
    pub(in ext2) fn sync_all(&self) -> Result<()> {
        if let Some(xattr) = &self.xattr {
            xattr.flush()?;
        }

        let fs = self.fs()?;
        let mut inner = self.inner.write();
        inner.sync_data_pages()?;

        inner.sync_indirect_blocks()?;

        inner.write_back_inode_desc(&fs, self.ino)
    }

    pub(in ext2) fn sync_data(&self) -> Result<()> {
        let fs = self.fs()?;
        let mut inner = self.inner.write();

        inner.sync_data_pages()?;

        inner.sync_indirect_blocks()?;

        inner.write_back_inode_desc(&fs, self.ino)
    }

    pub(super) fn try_reclaim_deleted_inode(&self) -> Result<bool> {
        if self.link_count() != 0 {
            return Ok(false);
        }

        let fs = self.fs()?;
        let group = fs.block_group(self.block_group_idx);
        if !group.is_inode_allocated(self.ino) {
            return Ok(false);
        }

        if let Some(xattr) = self.xattr.as_ref() {
            xattr.delete_xattr_block()?;
        }

        let mut inner = self.inner.write();
        let old_size = inner.file_size();
        let block_manager = inner.block_manager().ok().cloned();
        if block_manager.is_some() {
            inner.resize_page_cache(0, old_size)?;
        }
        inner.set_dtime(utils::now());
        inner.set_file_size(0);
        inner.set_file_acl(0);
        if inner.desc.sector_count > 0
            && let Some(block_manager) = block_manager
        {
            block_manager.truncate_to_byte_len(0);
        }
        inner.write_back_inode_desc(&fs, self.ino)?;

        fs.free_inode(self.ino, self.type_)?;
        Ok(true)
    }
}

impl InodeInner {
    pub(super) fn write_back_inode_desc(&mut self, fs: &Ext2, ino: Ext2Ino) -> Result<()> {
        if !self.is_dirty() {
            return Ok(());
        }

        let raw_block_ptrs = self.raw_block_ptrs();
        self.desc.block_ptrs = raw_block_ptrs.block_ptrs;
        self.desc.sector_count = raw_block_ptrs.sector_count;

        let raw_inode = RawInode::from(&*self.desc);
        fs.write_back_inode_desc(ino, &raw_inode)?;
        self.clear_dirty();
        Ok(())
    }

    fn sync_data_pages(&self) -> Result<()> {
        let file_size = self.file_size();
        if file_size == 0 {
            return Ok(());
        }

        match &self.payload {
            super::InodePayload::DataBacked { page_cache, .. } => {
                page_cache.flush_range(0..file_size)
            }
            _ => Ok(()),
        }
    }

    fn sync_indirect_blocks(&self) -> Result<()> {
        match &self.payload {
            super::InodePayload::DataBacked { block_manager, .. } => {
                block_manager.sync_indirect_blocks()
            }
            _ => Ok(()),
        }
    }
}
