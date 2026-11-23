use core::cmp::min;

use crate::{
    fs::{
        file::{InodeType, StatusFlags},
        utils::DirentVisitor,
        vfs::{
            inode::Inode,
            path::{self, Dentry},
        },
    },
    prelude::*,
};

const UPPER_LAYER_INDEX: usize = 0;

#[derive(Debug)]
pub(super) struct RealObject {
    layer_index: usize,
    dentry: Arc<Dentry>,
}

impl RealObject {
    pub(super) fn layer_index(&self) -> usize {
        self.layer_index
    }

    pub(super) fn dentry(&self) -> &Arc<Dentry> {
        &self.dentry
    }

    pub(super) fn real_inode(&self) -> &Arc<dyn Inode> {
        self.dentry.inode()
    }

    pub(super) fn status_flags_for_read(&self, status_flags: StatusFlags) -> StatusFlags {
        if self.layer_index == UPPER_LAYER_INDEX {
            status_flags
        } else {
            status_flags | StatusFlags::O_NOATIME
        }
    }

    pub(super) fn new_upper(dentry: Arc<Dentry>) -> Self {
        Self {
            layer_index: UPPER_LAYER_INDEX,
            dentry,
        }
    }

    pub(super) fn new_lower(layer_index: usize, dentry: Arc<Dentry>) -> Self {
        debug_assert!(layer_index != UPPER_LAYER_INDEX);
        Self {
            layer_index,
            dentry,
        }
    }

    pub(super) fn copy_metadata_to(&self, temp_dentry: &Dentry) -> Result<()> {
        let source_inode = self.real_inode();
        let temp_inode = temp_dentry.inode();
        temp_inode.set_owner(temp_dentry, source_inode.owner()?)?;
        temp_inode.set_group(temp_dentry, source_inode.group()?)?;
        if !matches!(source_inode.type_(), InodeType::SymLink) {
            temp_inode.set_mode(temp_dentry, source_inode.mode()?)?;
        }
        if source_inode.type_().is_regular_file() {
            temp_inode.resize(temp_dentry, source_inode.size())?;
        }
        Ok(())
    }

    pub(super) fn copy_data_to(&self, temp_dentry: &Dentry) -> Result<()> {
        const COPY_CHUNK_SIZE: usize = 64 * 1024;
        let size = self.real_inode().size();
        let mut offset = 0usize;
        let mut buffer = vec![0u8; COPY_CHUNK_SIZE];
        while offset < size {
            let chunk = min(COPY_CHUNK_SIZE, size - offset);
            let mut writer = VmWriter::from(&mut buffer[..chunk]).to_fallible();
            let read_len = self
                .real_inode()
                .read_at(offset, &mut writer, StatusFlags::empty())?;
            if read_len == 0 {
                break;
            }
            let mut reader = VmReader::from(&buffer[..read_len]).to_fallible();
            let write_len =
                temp_dentry
                    .inode()
                    .write_at(offset, &mut reader, StatusFlags::empty())?;
            if write_len != read_len {
                return Err(errno!(
                    EIO,
                    "the workdir temp accepted a short write during copy-up"
                ));
            }
            offset += write_len;
        }
        if offset != size {
            return Err(errno!(
                EIO,
                "the copy-up copied fewer bytes than the lower file declares"
            ));
        }
        Ok(())
    }

    pub(super) fn copy_timestamps_to(&self, temp_dentry: &Dentry) -> Result<()> {
        let source_inode = self.real_inode();
        let temp_inode = temp_dentry.inode();
        temp_inode.set_atime(temp_dentry, source_inode.atime());
        temp_inode.set_mtime(temp_dentry, source_inode.mtime());
        temp_inode.set_ctime(temp_dentry, source_inode.ctime());
        Ok(())
    }
}

#[derive(Debug)]
pub(super) struct RealObjectStack {
    pub(super) upper: Option<RealObject>,
    pub(super) lowers: Vec<RealObject>,
}

impl RealObjectStack {
    pub(super) fn new(upper: Option<RealObject>, lowers: Vec<RealObject>) -> Self {
        debug_assert!(upper.is_some() || !lowers.is_empty());
        Self { upper, lowers }
    }

    pub(super) fn upper_only(upper: RealObject) -> Self {
        Self {
            upper: Some(upper),
            lowers: Vec::new(),
        }
    }

    pub(super) fn lower_only(lower: RealObject) -> Self {
        Self {
            upper: None,
            lowers: vec![lower],
        }
    }

    pub(super) fn visible_source(&self) -> &RealObject {
        match &self.upper {
            Some(upper) => upper,
            None => &self.lowers[0],
        }
    }
}

pub(super) fn read_all_dirents(
    real_dir: &Arc<dyn Inode>,
    visitor: &mut dyn DirentVisitor,
) -> Result<()> {
    let mut offset = 0;
    loop {
        match real_dir.readdir_at(offset, visitor)? {
            0 => break,
            visited => offset += visited,
        }
    }
    Ok(())
}

pub(super) fn read_child_names(real_dir: &Arc<dyn Inode>) -> Result<Vec<String>> {
    let mut names: Vec<String> = Vec::new();
    read_all_dirents(real_dir, &mut names)?;
    names.retain(|name| !path::is_dot_or_dotdot(name));
    Ok(names)
}
