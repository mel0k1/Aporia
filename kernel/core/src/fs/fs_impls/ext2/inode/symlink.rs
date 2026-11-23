#![short_vis_path::add(ext2)]

use super::{
    super::Ext2, Inode, InodeInner, InodePayload, MAX_FAST_SYMLINK_LEN, RAW_BLOCK_PTRS_LEN,
    block_manager::RawBlockPtrs,
};
use crate::fs::ext2::{FilePerm, prelude::*, utils};

#[derive(Debug)]
pub(super) struct FastSymlinkTarget {
    block_ptrs: [u32; RAW_BLOCK_PTRS_LEN],
}

impl FastSymlinkTarget {
    pub(super) fn new(block_ptrs: [u32; RAW_BLOCK_PTRS_LEN]) -> Self {
        Self { block_ptrs }
    }

    fn new_zeros() -> Self {
        Self {
            block_ptrs: [0; RAW_BLOCK_PTRS_LEN],
        }
    }

    fn write(&mut self, target: &[u8]) -> Result<()> {
        debug_assert!(target.len() <= MAX_FAST_SYMLINK_LEN);
        self.block_ptrs.as_mut_bytes()[..target.len()].copy_from_slice(target);
        Ok(())
    }

    fn read(&self, len: usize) -> Vec<u8> {
        self.block_ptrs.as_bytes()[..len].to_vec()
    }

    pub(super) fn block_ptrs(&self) -> [u32; RAW_BLOCK_PTRS_LEN] {
        self.block_ptrs
    }
}

impl Inode {
    pub(in ext2) fn read_link(&self) -> Result<String> {
        if self.type_ != InodeType::SymLink {
            return Err(errno!(EINVAL));
        }

        let inner = self.inner.read();
        inner.read_link()
    }

    pub(in ext2) fn create_symlink(
        &self,
        name: &str,
        target: &str,
        perm: FilePerm,
    ) -> Result<Arc<Inode>> {
        if target.len() >= BLOCK_SIZE {
            return Err(errno!(ENAMETOOLONG));
        }

        self.create_initialized_child(name, InodeType::SymLink, perm, |fs, child| {
            let mut child_inner = child.inner.write();
            child_inner.write_symlink_target(fs, target)?;
            child_inner.set_mtime_ctime(utils::now());
            Ok(())
        })
    }
}

impl InodeInner {
    pub(super) fn is_fast_symlink(&self) -> bool {
        matches!(self.payload, InodePayload::FastSymlink { .. })
    }

    fn write_symlink_target(&mut self, fs: &Arc<Ext2>, target: &str) -> Result<()> {
        let target_len = target.len();

        if target.len() < MAX_FAST_SYMLINK_LEN {
            if let InodePayload::DataBacked { block_manager, .. } = &self.payload {
                block_manager.truncate_to_byte_len(0);
            }
            let mut fast_target = FastSymlinkTarget::new_zeros();
            fast_target.write(target.as_bytes())?;
            self.payload = InodePayload::FastSymlink {
                target: fast_target,
            };
        } else {
            if !matches!(self.payload, InodePayload::DataBacked { .. }) {
                let raw_block_ptrs = RawBlockPtrs::new(
                    InodePayload::xattr_sectors(self.desc.file_acl),
                    [0; RAW_BLOCK_PTRS_LEN],
                );
                self.payload = InodePayload::new_data_backed(
                    self.file_size(),
                    raw_block_ptrs,
                    Arc::downgrade(fs),
                );
            }
            self.prepare_write(fs.as_ref(), 0, target_len)?;
            self.page_cache().write_bytes(0, target.as_bytes())?;
        }

        self.set_file_size(target_len);
        Ok(())
    }

    fn read_link(&self) -> Result<String> {
        let link_size = self.file_size();

        if let InodePayload::FastSymlink { target } = &self.payload {
            let read_len = link_size.min(MAX_FAST_SYMLINK_LEN - 1);
            let target_bytes = target.read(read_len);
            return String::from_utf8(target_bytes)
                .map_err(|_| errno!(EIO, "symlink target is not valid UTF-8"));
        }

        let mut target = vec![0u8; link_size];
        self.page_cache()
            .read_bytes(0, &mut target)
            .map_err(|_| errno!(EIO, "failed to read symlink target from page cache"))?;

        String::from_utf8(target).map_err(|_| errno!(EIO, "symlink target is not valid UTF-8"))
    }
}
