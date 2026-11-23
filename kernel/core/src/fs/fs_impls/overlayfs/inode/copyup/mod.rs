use self::workdir::WorkdirTemp;
use crate::{
    fs::{
        file::{InodeType, SyncMode},
        fs_impls::overlayfs::{
            fs::OverlayFs,
            inode::{CreateOp, OverlayInode, OverlayXattrType},
            real::RealObject,
        },
        vfs::{
            inode::{Inode, MknodType, RenameMode, SymbolicLink},
            path::Dentry,
        },
    },
    prelude::*,
};

pub(super) mod workdir;

impl OverlayFs {
    pub(super) fn copy_up_at(&self, dentry: &Dentry) -> Result<()> {
        let promoted = Arc::downcast::<OverlayInode>(dentry.inode().clone())
            .expect("the copy-up operation dentry is not an overlay inode");

        let mut chain: Vec<(Arc<OverlayInode>, String)> = Vec::new();
        let mut object = promoted;
        let mut name = dentry.name();
        let mut parent_dentry = dentry.parent();
        let mut publication_parent = loop {
            let Some(parent) = parent_dentry else {
                return Err(errno!(
                    EIO,
                    "the copy-up operation dentry has no overlay parent"
                ));
            };
            let parent_object = Arc::downcast::<OverlayInode>(parent.inode().clone())
                .expect("the copy-up publication parent is not an overlay inode");
            chain.push((object, name));
            if parent_object.upper.get().is_some() {
                break parent_object;
            }
            object = parent_object;
            name = parent.name();
            parent_dentry = parent.parent();
        };

        for (promoted_object, name) in chain.iter().rev() {
            self.promote_level(&publication_parent, promoted_object, name)?;
            publication_parent = Arc::clone(promoted_object);
        }
        Ok(())
    }

    pub(super) fn promote_rename_target(
        &self,
        publication_parent: &Arc<OverlayInode>,
        promoted_object: &Arc<OverlayInode>,
        name: &str,
    ) -> Result<()> {
        if self.policy().is_effective_read_only() {
            return Err(errno!(EROFS, "the overlay mount is read-only"));
        }
        self.promote_level(publication_parent, promoted_object, name)
    }

    fn promote_level(
        &self,
        publication_parent: &Arc<OverlayInode>,
        promoted_object: &Arc<OverlayInode>,
        name: &str,
    ) -> Result<()> {
        let _parent_guard = publication_parent.lock();
        let _object_guard = promoted_object.lock();

        if promoted_object.is_name_taken() {
            return Err(errno!(
                ENOENT,
                "the copy-up target name was taken away before the promotion round"
            ));
        }
        if publication_parent.is_name_taken() {
            return Err(errno!(
                ENOENT,
                "the copy-up publication parent is no longer reachable"
            ));
        }

        let upper_parent = publication_parent.writable_upper();

        let upper_dir = upper_parent.dentry().as_dir_dentry_or_err()?;
        let existing = match upper_dir.lookup_child(name) {
            Ok(child) => Some(child),
            Err(err) if err.error() == Errno::ENOENT => None,
            Err(err) => return Err(err),
        };
        match (&existing, promoted_object.upper.get()) {
            (Some(child), Some(ours)) if Arc::ptr_eq(child.inode(), ours.real_inode()) => {
                return Ok(());
            }
            (None, Some(_)) => return Ok(()),
            (Some(_), _) => {
                return Err(errno!(
                    ESTALE,
                    "the promotion name is already held in the upper"
                ));
            }
            (None, None) => {}
        }

        let source = promoted_object.real_object();
        let staged = promoted_object.stage_in_workdir(name)?;
        let origin_record_landed = promoted_object.stage_temp_contents(&staged, source)?;
        self.commit_copyup_publication(
            promoted_object,
            name,
            upper_parent.dentry(),
            staged,
            origin_record_landed,
        )?;
        Ok(())
    }

    fn commit_copyup_publication(
        &self,
        promoted_object: &Arc<OverlayInode>,
        name: &str,
        upper_parent: &Arc<Dentry>,
        staged: WorkdirTemp,
        origin_record_landed: bool,
    ) -> Result<()> {
        let namespace = self.policy().xattr_namespace();
        if let Ok(false) = OverlayXattrType::Impure.is_positive_on(upper_parent, namespace) {
            let _ = OverlayXattrType::Impure.set_value_on(upper_parent, namespace, None);
        }
        let published = staged.dentry().clone();
        staged.publish(upper_parent, name, RenameMode::Replace)?;
        let published_upper = RealObject::new_upper(published);
        let upper_key = self.identity().real_id_of(&published_upper);
        promoted_object.upper.call_once(|| published_upper);
        if !origin_record_landed
            && !promoted_object.type_().is_directory()
            && let Some(lower) = promoted_object.lowers.first()
        {
            self.inodes().rekey(
                self.identity().real_id_of(lower),
                upper_key,
                promoted_object,
            );
        }
        Ok(())
    }
}

impl OverlayInode {
    fn stage_in_workdir(&self, name: &str) -> Result<WorkdirTemp> {
        let fs = self.fs_arc();
        let upper_workdir = fs.upper_workdir_inuse();
        let source = self.real_object();
        let temp = match source.real_inode().type_() {
            InodeType::Dir => {
                let mode = source.real_inode().mode()?;
                upper_workdir.create_workdir_temp(
                    name,
                    CreateOp::General {
                        kind: InodeType::Dir,
                        mode,
                    },
                )?
            }
            InodeType::File => {
                let mode = source.real_inode().mode()?;
                upper_workdir.create_workdir_temp(
                    name,
                    CreateOp::General {
                        kind: InodeType::File,
                        mode,
                    },
                )?
            }
            InodeType::SymLink => {
                let mode = source.real_inode().mode()?;
                let target = match source.real_inode().read_link()? {
                    SymbolicLink::Plain(target) => target,
                    SymbolicLink::Path(_) => {
                        return Err(errno!(
                            EOPNOTSUPP,
                            "a path-style symlink target cannot be copied up"
                        ));
                    }
                };
                upper_workdir.create_workdir_temp(
                    name,
                    CreateOp::Symlink {
                        target: &target,
                        mode,
                    },
                )?
            }
            InodeType::CharDevice
            | InodeType::BlockDevice
            | InodeType::NamedPipe
            | InodeType::Socket => {
                let mknod_type = match source.real_inode().type_() {
                    InodeType::NamedPipe => MknodType::NamedPipe,
                    InodeType::CharDevice => {
                        let rdev = source
                            .real_inode()
                            .metadata()?
                            .self_dev_id
                            .ok_or_else(|| {
                                errno!(EINVAL, "the lower char device has no device ID",)
                            })?
                            .as_encoded_u64();
                        MknodType::CharDevice(rdev)
                    }
                    InodeType::BlockDevice => {
                        let rdev = source
                            .real_inode()
                            .metadata()?
                            .self_dev_id
                            .ok_or_else(|| {
                                errno!(EINVAL, "the lower block device has no device ID",)
                            })?
                            .as_encoded_u64();
                        MknodType::BlockDevice(rdev)
                    }
                    _ => {
                        return Err(errno!(EOPNOTSUPP, "socket nodes cannot be copied up"));
                    }
                };
                let mode = source.real_inode().mode()?;
                upper_workdir.create_workdir_temp(
                    name,
                    CreateOp::Mknod {
                        mode,
                        node: &mknod_type,
                    },
                )?
            }
            InodeType::Unknown => {
                return Err(errno!(
                    EINVAL,
                    "overlay objects of an unknown type cannot be promoted"
                ));
            }
        };
        Ok(temp)
    }

    fn stage_temp_contents(&self, temp: &WorkdirTemp, source: &RealObject) -> Result<bool> {
        source.copy_metadata_to(temp.dentry())?;

        self.copy_eligible_xattrs(source, temp, false)?;

        if temp.inode().type_().is_regular_file() {
            source.copy_data_to(temp.dentry())?;
        }

        let origin_record_landed = if source.real_inode().type_().is_directory()
            || source.real_inode().metadata()?.nr_hard_links <= 1
        {
            self.fs_arc().record_origin(temp, source)?
        } else {
            false
        };

        source.copy_timestamps_to(temp.dentry())?;

        if temp.inode().type_().is_regular_file() {
            temp.inode().sync(SyncMode::Full)?;
        }

        Ok(origin_record_landed)
    }
}
