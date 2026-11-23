#![short_vis_path::add(overlayfs)]

use core::sync::atomic::{AtomicBool, Ordering};

use crate::{
    fs::{
        file::{InodeMode, InodeType},
        fs_impls::overlayfs::{
            fs::{
                OverlayFs,
                mount::{WhiteoutCapability, inuse::UpperWorkdirInuse},
            },
            inode::{CreateOp, copyup::workdir::WorkdirTemp, xattr::OverlayXattrType},
            real::read_child_names,
        },
        vfs::{
            inode::{MknodType, RenameMode},
            path::Dentry,
            xattr::XattrNamespace,
        },
    },
    prelude::*,
};

#[derive(Debug)]
pub(in overlayfs) struct WhiteoutCache {
    shared: Option<WhiteoutHandle>,
    can_share_by_link: AtomicBool,
}

impl WhiteoutCache {
    pub(in overlayfs) fn new() -> Self {
        Self {
            shared: None,
            can_share_by_link: AtomicBool::new(true),
        }
    }

    pub(in overlayfs) fn with_shared_whiteout(
        capability: WhiteoutCapability,
        upper_workdir: &UpperWorkdirInuse,
        namespace: XattrNamespace,
    ) -> Self {
        let shared = upper_workdir.stage_whiteout(capability, namespace).ok();
        Self {
            shared,
            can_share_by_link: AtomicBool::new(true),
        }
    }

    fn shared_whiteout(&self) -> Option<&WhiteoutHandle> {
        if self.can_share_by_link.load(Ordering::Relaxed) {
            self.shared.as_ref()
        } else {
            None
        }
    }

    fn latch_degraded(&self) {
        self.can_share_by_link.store(false, Ordering::Relaxed);
    }
}

#[derive(Debug)]
struct WhiteoutHandle {
    workdir_name: String,
    dentry: Arc<Dentry>,
}

impl UpperWorkdirInuse {
    fn stage_whiteout(
        &self,
        capability: WhiteoutCapability,
        namespace: XattrNamespace,
    ) -> Result<WhiteoutHandle> {
        const WHITEOUT_TEMP_NAME_COMPONENT: &str = "whiteout";
        match capability {
            WhiteoutCapability::CharDevice => {
                const WHITEOUT_CHAR_DEV: u64 = 0;
                let temp = self.create_workdir_temp(
                    WHITEOUT_TEMP_NAME_COMPONENT,
                    CreateOp::Mknod {
                        mode: InodeMode::empty(),
                        node: &MknodType::CharDevice(WHITEOUT_CHAR_DEV),
                    },
                )?;
                Ok(temp.into_whiteout())
            }
            WhiteoutCapability::Xattr => {
                let temp = self.create_workdir_temp(
                    WHITEOUT_TEMP_NAME_COMPONENT,
                    CreateOp::general(InodeType::File, InodeMode::empty()),
                )?;
                OverlayXattrType::Whiteout.set_value_on(temp.dentry(), namespace, None)?;
                Ok(temp.into_whiteout())
            }
            WhiteoutCapability::Unsupported => Err(errno!(
                EOPNOTSUPP,
                "the upper filesystem supports no whiteout form",
            )),
        }
    }
}

impl WorkdirTemp {
    fn into_whiteout(mut self) -> WhiteoutHandle {
        self.mark_consumed();
        WhiteoutHandle {
            workdir_name: self.name().into(),
            dentry: self.dentry().clone(),
        }
    }
}

impl OverlayFs {
    pub(in super::super) fn is_whiteout(&self, real_dentry: &Dentry) -> Result<bool> {
        let metadata = real_dentry.inode().metadata()?;
        if metadata.type_ == InodeType::CharDevice
            && metadata.self_dev_id.is_none_or(|dev_id| dev_id.is_null())
        {
            return Ok(true);
        }
        if metadata.type_ != InodeType::File || metadata.size != 0 {
            return Ok(false);
        }
        OverlayXattrType::Whiteout.is_positive_on(real_dentry, self.policy().xattr_namespace())
    }

    pub(super) fn publish_whiteout(
        &self,
        upper_parent: &Arc<Dentry>,
        name: &str,
        replace_target: Option<InodeType>,
    ) -> Result<()> {
        if replace_target.is_none()
            && let Some(shared) = self.whiteout_cache().shared_whiteout()
        {
            let parent_dir = upper_parent.as_dir_dentry_or_err()?;
            match parent_dir.link(&shared.dentry, name) {
                Ok(()) => return Ok(()),
                Err(err) => {
                    if matches!(err.error(), Errno::EMLINK | Errno::EOPNOTSUPP) {
                        self.whiteout_cache().latch_degraded();
                    }
                }
            }
        }

        let handle = self.create_whiteout_temp()?;
        let workspace = self.upper_workdir_inuse().workdir_workspace()?;
        let workdir_dir = workspace.as_dir_dentry_or_err()?;
        let upper_dir = upper_parent.as_dir_dentry_or_err()?;
        match replace_target {
            Some(target_type) if target_type.is_directory() => {
                workdir_dir.rename(&handle.workdir_name, &upper_dir, name, RenameMode::Exchange)?;
                if let Err(err) = workdir_dir.rmdir(&handle.workdir_name) {
                    warn!(
                        "whiteout publish: workdir cleanup of the displaced directory {:?} failed: {:?}",
                        handle.workdir_name, err
                    );
                }
            }
            _ => {
                workdir_dir.rename(&handle.workdir_name, &upper_dir, name, RenameMode::Replace)?;
            }
        }
        Ok(())
    }

    fn create_whiteout_temp(&self) -> Result<WhiteoutHandle> {
        self.upper_workdir_inuse().stage_whiteout(
            self.policy().whiteout_capability(),
            self.policy().xattr_namespace(),
        )
    }

    pub(super) fn remove_whiteout(&self, upper_parent: &Arc<Dentry>, name: &str) -> Result<()> {
        let upper_dir = upper_parent.as_dir_dentry_or_err()?;
        let holds_whiteout = match upper_dir.lookup_child(name) {
            Ok(child) => self.is_whiteout(&child)?,
            Err(err) if err.error() == Errno::ENOENT => false,
            Err(err) => return Err(err),
        };
        if !holds_whiteout {
            return Err(errno!(
                EIO,
                "the cleared name no longer holds the whiteout the exchange displaced"
            ));
        }
        upper_dir.unlink(name)?;
        Ok(())
    }

    pub(super) fn sweep_whiteouts(&self, upper_dir: &Arc<Dentry>) -> Result<()> {
        let names = read_child_names(upper_dir.inode())?;
        self.validate_whiteout_children(upper_dir, &names)?;
        let dir = upper_dir.as_dir_dentry_or_err()?;
        for name in &names {
            dir.unlink(name)?;
        }
        Ok(())
    }

    fn validate_whiteout_children(&self, upper_dir: &Arc<Dentry>, names: &[String]) -> Result<()> {
        let dir = upper_dir.as_dir_dentry_or_err()?;
        for name in names {
            let child = dir.lookup_child(name)?;
            if !self.is_whiteout(&child)? {
                return Err(errno!(
                    ENOTEMPTY,
                    "a hidden non-whiteout entry prevents the overlay directory removal"
                ));
            }
        }
        Ok(())
    }
}
