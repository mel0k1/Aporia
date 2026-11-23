use core::time::Duration;

use super::OverlayInode;
use crate::{
    fs::{file::InodeMode, vfs::path::Dentry},
    prelude::*,
    process::{Gid, Uid},
};

impl OverlayInode {
    pub(super) fn set_mode_impl(&self, self_dentry: &Dentry, mode: InodeMode) -> Result<()> {
        let upper = self.writable_real_object(self_dentry)?;
        upper.real_inode().set_mode(upper.dentry(), mode)
    }

    pub(super) fn set_owner_impl(&self, self_dentry: &Dentry, uid: Uid) -> Result<()> {
        let upper = self.writable_real_object(self_dentry)?;
        upper.real_inode().set_owner(upper.dentry(), uid)
    }

    pub(super) fn set_group_impl(&self, self_dentry: &Dentry, gid: Gid) -> Result<()> {
        let upper = self.writable_real_object(self_dentry)?;
        upper.real_inode().set_group(upper.dentry(), gid)
    }

    pub(super) fn set_atime_impl(&self, self_dentry: &Dentry, time: Duration) {
        if let Ok(upper) = self.writable_real_object(self_dentry) {
            upper.real_inode().set_atime(upper.dentry(), time);
        }
    }

    pub(super) fn set_mtime_impl(&self, self_dentry: &Dentry, time: Duration) {
        if let Ok(upper) = self.writable_real_object(self_dentry) {
            upper.real_inode().set_mtime(upper.dentry(), time);
        }
    }

    pub(super) fn set_ctime_impl(&self, self_dentry: &Dentry, time: Duration) {
        if let Ok(upper) = self.writable_real_object(self_dentry) {
            upper.real_inode().set_ctime(upper.dentry(), time);
        }
    }
}
