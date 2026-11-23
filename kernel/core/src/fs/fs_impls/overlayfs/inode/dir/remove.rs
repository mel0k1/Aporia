use crate::{
    fs::{
        file::InodeType,
        fs_impls::overlayfs::{
            inode::{
                CreateOp, Lookup, OverlayInode, OverlayInodeLockGuard, OverlayXattrType,
                copyup::workdir::WorkdirTemp,
            },
            real::{RealObject, read_child_names},
        },
        vfs::{
            inode::{Inode, RenameMode},
            path::Dentry,
        },
    },
    prelude::*,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RemoveKind {
    Unlink,
    Rmdir,
}

#[derive(Clone, Copy, Debug)]
enum RemoveAction {
    DeletePureUpper,
    WhiteoutLowerOnly,
    ReplaceUpperWithWhiteout,
}

struct RemoveTarget {
    inode: Arc<OverlayInode>,
    name: String,
}

impl OverlayInode {
    pub(in super::super) fn unlink_impl(&self, child_dentry: &Dentry) -> Result<()> {
        let parent_dentry = child_dentry
            .parent()
            .expect("the unlinked child has no parent dentry");
        self.refuse_removal_early(child_dentry, RemoveKind::Unlink)?;
        self.writable_real_object(&parent_dentry)?;
        self.remove_target(child_dentry, RemoveKind::Unlink)
    }

    pub(in super::super) fn rmdir_impl(&self, child_dentry: &Dentry) -> Result<()> {
        let parent_dentry = child_dentry
            .parent()
            .expect("the removed directory has no parent dentry");
        self.refuse_removal_early(child_dentry, RemoveKind::Rmdir)?;
        self.writable_real_object(&parent_dentry)?;
        self.remove_target(child_dentry, RemoveKind::Rmdir)
    }

    fn refuse_removal_early(&self, child_dentry: &Dentry, kind: RemoveKind) -> Result<()> {
        let target = self.resolve_remove_target(child_dentry)?;
        let target_guard = target.inode.lock();
        let _ = self.decide_remove_action(&target, kind, &target_guard)?;
        Ok(())
    }

    fn remove_target(&self, child_dentry: &Dentry, kind: RemoveKind) -> Result<()> {
        let mut dir_guard = self.lock();
        let target = self.resolve_remove_target(child_dentry)?;
        let target_guard = target.inode.lock();
        let action = self.decide_remove_action(&target, kind, &target_guard)?;
        match action {
            RemoveAction::DeletePureUpper => self.delete_pure_upper(&target, kind, &mut dir_guard),
            RemoveAction::WhiteoutLowerOnly => self.whiteout_lower_only(&target, &mut dir_guard),
            RemoveAction::ReplaceUpperWithWhiteout => {
                self.replace_upper_with_whiteout(&target, kind, &mut dir_guard)
            }
        }
    }

    fn resolve_remove_target(&self, child_dentry: &Dentry) -> Result<RemoveTarget> {
        let name = child_dentry.name();
        let lookup = self.fs_arc().lookup(self, &name)?;
        let inode = match lookup {
            Lookup::Positive(inode) => inode,
            Lookup::Negative(_) => return Err(errno!(ENOENT)),
        };
        if inode.is_stale_upper_target(child_dentry) {
            return Err(errno!(
                ESTALE,
                "the upper object at the target name became stale"
            ));
        }
        Ok(RemoveTarget { inode, name })
    }

    fn decide_remove_action(
        &self,
        target: &RemoveTarget,
        kind: RemoveKind,
        target_guard: &OverlayInodeLockGuard<'_>,
    ) -> Result<RemoveAction> {
        let fs = self.fs_arc();
        let namespace = fs.policy().xattr_namespace();
        let target_upper = target.inode.upper.get();
        let lower_positive = if target_upper.is_some() {
            match self.lower_entry(&target.name) {
                Ok(found) => found.is_some(),
                Err(_) => true,
            }
        } else {
            false
        };
        let is_pure_upper = match target_upper {
            Some(upper_obj) => {
                !lower_positive
                    && !(target.inode.type_().is_directory()
                        && OverlayXattrType::Opaque
                            .is_positive_on(upper_obj.dentry(), namespace)?)
            }
            None => false,
        };

        if kind == RemoveKind::Rmdir {
            if !target.inode.type_().is_directory() {
                return Err(errno!(
                    ENOTDIR,
                    "a non-directory cannot be removed as a directory"
                ));
            }
            if lower_positive || !is_pure_upper {
                if !target.inode.is_empty_dir(target_guard)? {
                    return Err(errno!(ENOTEMPTY, "the overlay directory is not empty"));
                }
            }
        } else if target.inode.type_().is_directory() {
            return Err(errno!(EISDIR, "a directory cannot be unlinked"));
        }

        if !is_pure_upper && !fs.policy().can_express_whiteout() {
            return Err(errno!(
                EOPNOTSUPP,
                "the upper filesystem supports no whiteout form; the removal \
                 cannot publish one"
            ));
        }
        Ok(if is_pure_upper {
            RemoveAction::DeletePureUpper
        } else if target_upper.is_some() {
            RemoveAction::ReplaceUpperWithWhiteout
        } else {
            RemoveAction::WhiteoutLowerOnly
        })
    }

    fn delete_pure_upper(
        &self,
        target: &RemoveTarget,
        kind: RemoveKind,
        dir_guard: &mut OverlayInodeLockGuard<'_>,
    ) -> Result<()> {
        let fs = self.fs_arc();
        let target_upper = target.inode.upper.get();
        let upper_parent = self.writable_upper();
        if kind == RemoveKind::Rmdir {
            let target_upper_dir = target_upper.ok_or_else(|| {
                errno!(
                    EIO,
                    "the pure-upper rmdir target has no upper real directory",
                )
            })?;
            fs.sweep_whiteouts(target_upper_dir.dentry())?;
        }
        let upper_dir = upper_parent.dentry().as_dir_dentry_or_err()?;
        let result = if kind == RemoveKind::Rmdir {
            upper_dir.rmdir(&target.name)
        } else {
            upper_dir.unlink(&target.name)
        };
        result.map_err(translate_stale_upper_enoent)?;
        target.inode.mark_name_taken();
        **dir_guard = None;
        Ok(())
    }

    fn whiteout_lower_only(
        &self,
        target: &RemoveTarget,
        dir_guard: &mut OverlayInodeLockGuard<'_>,
    ) -> Result<()> {
        let fs = self.fs_arc();
        let upper_parent = self.writable_upper().dentry();
        let published = fs.publish_whiteout(upper_parent, &target.name, None);
        if published.is_ok() {
            target.inode.mark_name_taken();
            **dir_guard = None;
        }
        published
    }

    fn replace_upper_with_whiteout(
        &self,
        target: &RemoveTarget,
        kind: RemoveKind,
        dir_guard: &mut OverlayInodeLockGuard<'_>,
    ) -> Result<()> {
        let fs = self.fs_arc();
        let upper_parent = self.writable_upper().dentry();
        let occupant = target
            .inode
            .upper
            .get()
            .expect("an upper entry replaced by a whiteout has an upper real object");

        let cleared =
            kind == RemoveKind::Rmdir && !read_child_names(occupant.real_inode())?.is_empty();
        if cleared {
            let empty_copy = fs.upper_workdir_inuse().create_workdir_temp(
                &target.name,
                CreateOp::General {
                    kind: InodeType::Dir,
                    mode: occupant.real_inode().mode()?,
                },
            )?;

            let empty_copy_name = String::from(empty_copy.name());

            self.exchange_occupant_for_empty_copy(target, occupant, upper_parent, empty_copy)?;

            let workspace = fs.upper_workdir_inuse().workdir_workspace()?;
            let workdir_dir = workspace.as_dir_dentry_or_err()?;
            let cleanup_failure = match workdir_dir.lookup_child(&empty_copy_name) {
                Ok(displaced) => {
                    let sweep_failure = fs.sweep_whiteouts(&displaced).err();
                    let rmdir_failure = workdir_dir.rmdir(&empty_copy_name).err();
                    sweep_failure.or(rmdir_failure)
                }
                Err(err) => Some(err),
            };
            if let Some(err) = cleanup_failure {
                warn!(
                    "clear-empty: the displaced-directory cleanup left residue: {:?}",
                    err
                );
            }
        }

        let published = fs
            .publish_whiteout(upper_parent, &target.name, Some(target.inode.type_()))
            .map_err(translate_stale_upper_enoent);
        if cleared || published.is_ok() {
            target.inode.mark_name_taken();
            **dir_guard = None;
        }
        published
    }

    fn exchange_occupant_for_empty_copy(
        &self,
        target: &RemoveTarget,
        occupant: &RealObject,
        upper_parent: &Arc<Dentry>,
        empty_copy: WorkdirTemp,
    ) -> Result<()> {
        let fs = self.fs_arc();
        let namespace = fs.policy().xattr_namespace();
        if !fs.policy().can_store_private_xattr() {
            return Err(errno!(
                EOPNOTSUPP,
                "the upper filesystem cannot store the opaque marker \
                 required for the clear-empty directory exchange"
            ));
        }
        OverlayXattrType::Opaque.set_value_on(empty_copy.dentry(), namespace, None)?;
        self.copy_eligible_xattrs(occupant, &empty_copy, true)?;
        occupant.copy_metadata_to(empty_copy.dentry())?;
        occupant.copy_timestamps_to(empty_copy.dentry())?;
        empty_copy
            .publish(upper_parent, &target.name, RenameMode::Exchange)
            .map_err(translate_stale_upper_enoent)?;
        Ok(())
    }
}

fn translate_stale_upper_enoent(err: Error) -> Error {
    if err.error() == Errno::ENOENT {
        errno!(ESTALE, "the upper object at the target name became stale",)
    } else {
        err
    }
}
