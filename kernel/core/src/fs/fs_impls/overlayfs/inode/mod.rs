mod copyup;
mod data;
mod dir;
mod identity;
mod inode_cache;
mod lookup;
mod metadata;
mod open;
mod readdir;
mod xattr;

use core::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use spin::Once;

pub(super) use self::{
    dir::whiteout::WhiteoutCache, identity::IdentityPolicy, inode_cache::InodeCache,
    xattr::OverlayXattrType,
};
use self::{
    identity::ObjectVisibleId,
    lookup::{Lookup, NegativeLookup},
    readdir::{OverlayInodeLockGuard, OverlayInodeLockPayload},
};
use crate::{
    fs::{
        file::{AccessMode, InodeMode, InodeType, PerOpenFileOps, StatusFlags, SyncMode},
        fs_impls::overlayfs::{fs::OverlayFs, real::RealObject},
        vfs::{
            file_system::FileSystem,
            inode::{
                Extension, FallocMode, FileOps, Inode, Metadata, MknodType, RenameMode,
                SymbolicLink,
            },
            path::Dentry,
            xattr::{XattrName, XattrNamespace, XattrSetFlags},
        },
    },
    prelude::*,
    process::{Gid, Uid},
    vm::page_cache::Vmo,
};

pub(super) struct OverlayInode {
    fs: Weak<OverlayFs>,
    lowers: Vec<RealObject>,
    upper: Once<RealObject>,
    name_taken: AtomicBool,
    object_id: ObjectVisibleId,
    lock: Mutex<OverlayInodeLockPayload>,
    extension: Extension,
    this: Weak<OverlayInode>,
}

impl OverlayInode {
    fn same_directory_as(&self, other: &OverlayInode) -> bool {
        debug_assert!(self.type_().is_directory() && other.type_().is_directory());
        self.object_id == other.object_id
    }

    fn real_object(&self) -> &RealObject {
        match self.upper.get() {
            Some(upper) => upper,
            None => self
                .lowers
                .first()
                .expect("a real-object stack is never empty"),
        }
    }

    fn writable_real_object(&self, copyup_dentry: &Dentry) -> Result<&RealObject> {
        let fs = self.fs_arc();
        if fs.policy().is_effective_read_only() {
            return Err(errno!(EROFS, "the overlay mount is read-only"));
        }
        if let Some(upper) = self.upper.get() {
            return Ok(upper);
        }
        fs.copy_up_at(copyup_dentry)?;
        Ok(self.writable_upper())
    }

    fn mark_name_taken(&self) {
        self.name_taken.store(true, Ordering::Relaxed);
    }

    fn is_name_taken(&self) -> bool {
        self.name_taken.load(Ordering::Relaxed)
    }

    fn writable_upper(&self) -> &RealObject {
        self.upper
            .get()
            .expect("an overlay object on a write path has an upper real object")
    }

    fn lock(&self) -> OverlayInodeLockGuard<'_> {
        Mutex::lock(&self.lock)
    }

    fn fs_arc(&self) -> Arc<OverlayFs> {
        self.fs
            .upgrade()
            .expect("the owning fs outlives every overlay inode")
    }

    fn self_arc(&self) -> Arc<OverlayInode> {
        self.this
            .upgrade()
            .expect("the overlay inode is still owned by an Arc")
    }
}

#[derive(Clone, Copy)]
enum CreateOp<'a> {
    General {
        kind: InodeType,
        mode: InodeMode,
    },
    Symlink {
        target: &'a str,
        mode: InodeMode,
    },
    Mknod {
        mode: InodeMode,
        node: &'a MknodType,
    },
}

impl<'a> CreateOp<'a> {
    fn general(kind: InodeType, mode: InodeMode) -> Self {
        debug_assert!(
            kind != InodeType::SymLink,
            "overlay symlinks are created only through create_symlink",
        );
        Self::General { kind, mode }
    }

    fn symlink(target: &'a str, mode: InodeMode) -> Self {
        Self::Symlink { target, mode }
    }

    fn mknod(node: &'a MknodType, mode: InodeMode) -> Result<Self> {
        if matches!(node, MknodType::CharDevice(0)) {
            return Err(errno!(
                EPERM,
                "a raw 0:0 whiteout char device must not be user-creatable"
            ));
        }
        Ok(Self::Mknod { mode, node })
    }

    fn object_type(&self) -> InodeType {
        match self {
            Self::General { kind, .. } => *kind,
            Self::Symlink { .. } => InodeType::SymLink,
            Self::Mknod { node, .. } => match *node {
                MknodType::NamedPipe => InodeType::NamedPipe,
                MknodType::CharDevice(_) => InodeType::CharDevice,
                MknodType::BlockDevice(_) => InodeType::BlockDevice,
            },
        }
    }

    fn create_child_in(&self, dir: &Dentry, name: &str) -> Result<Arc<Dentry>> {
        let dir = dir.as_dir_dentry_or_err()?;
        match self {
            Self::General { kind, mode } => {
                dir.create_child(name, || dir.inode().create(&dir, name, *kind, *mode))
            }
            Self::Symlink { target, mode } => dir.create_child(name, || {
                dir.inode().create_symlink(&dir, name, target, *mode)
            }),
            Self::Mknod { mode, node } => dir.mknod(
                name,
                *mode,
                match *node {
                    MknodType::NamedPipe => MknodType::NamedPipe,
                    MknodType::CharDevice(device) => MknodType::CharDevice(*device),
                    MknodType::BlockDevice(device) => MknodType::BlockDevice(*device),
                },
            ),
        }
    }
}

impl FileOps for OverlayInode {
    fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        self.read_at_impl(offset, writer, status_flags)
    }

    fn write_at(
        &self,
        offset: usize,
        reader: &mut VmReader,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        self.write_at_impl(offset, reader, status_flags)
    }
}

impl Inode for OverlayInode {
    fn size(&self) -> usize {
        self.real_object().real_inode().size()
    }

    fn metadata(&self) -> Result<Metadata> {
        let mut metadata = self.real_object().real_inode().metadata()?;
        metadata.ino = self.object_id.ino;
        metadata.container_dev_id = self.object_id.dev;
        Ok(metadata)
    }

    fn ino(&self) -> u64 {
        self.object_id.ino
    }

    fn type_(&self) -> InodeType {
        self.real_object().real_inode().type_()
    }

    fn mode(&self) -> Result<InodeMode> {
        self.real_object().real_inode().mode()
    }

    fn owner(&self) -> Result<Uid> {
        self.real_object().real_inode().owner()
    }

    fn group(&self) -> Result<Gid> {
        self.real_object().real_inode().group()
    }

    fn atime(&self) -> Duration {
        self.real_object().real_inode().atime()
    }

    fn mtime(&self) -> Duration {
        self.real_object().real_inode().mtime()
    }

    fn ctime(&self) -> Duration {
        self.real_object().real_inode().ctime()
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        let _dir_guard = self.lock();
        if !self.type_().is_directory() {
            return Err(errno!(
                ENOTDIR,
                "lookup is supported on overlay directories only"
            ));
        }
        let fs = self
            .fs
            .upgrade()
            .ok_or_else(|| errno!(EIO, "the overlay mount is no longer alive"))?;
        match fs.lookup(self, name)? {
            Lookup::Positive(inode) => Ok(inode),
            Lookup::Negative(_) => Err(errno!(ENOENT)),
        }
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        self.fs_arc()
    }

    fn extension(&self) -> &Extension {
        &self.extension
    }

    fn open(
        &self,
        self_dentry: &Dentry,
        access_mode: AccessMode,
        status_flags: StatusFlags,
    ) -> Option<Result<Box<dyn PerOpenFileOps>>> {
        self.open_impl(self_dentry, access_mode, status_flags)
    }

    fn seek_end(&self) -> Option<usize> {
        self.seek_end_impl()
    }

    fn resize(&self, self_dentry: &Dentry, new_size: usize) -> Result<()> {
        self.resize_impl(self_dentry, new_size)
    }

    fn fallocate(&self, mode: FallocMode, offset: usize, len: usize) -> Result<()> {
        self.fallocate_impl(mode, offset, len)
    }

    fn sync(&self, mode: SyncMode) -> Result<()> {
        self.sync_impl(mode)
    }

    fn read_link(&self) -> Result<SymbolicLink> {
        self.read_link_impl()
    }

    fn page_cache(&self) -> Option<Arc<Vmo>> {
        self.page_cache_impl()
    }

    fn set_mode(&self, self_dentry: &Dentry, mode: InodeMode) -> Result<()> {
        self.set_mode_impl(self_dentry, mode)
    }

    fn set_owner(&self, self_dentry: &Dentry, uid: Uid) -> Result<()> {
        self.set_owner_impl(self_dentry, uid)
    }

    fn set_group(&self, self_dentry: &Dentry, gid: Gid) -> Result<()> {
        self.set_group_impl(self_dentry, gid)
    }

    fn set_atime(&self, self_dentry: &Dentry, time: Duration) {
        self.set_atime_impl(self_dentry, time)
    }

    fn set_mtime(&self, self_dentry: &Dentry, time: Duration) {
        self.set_mtime_impl(self_dentry, time)
    }

    fn set_ctime(&self, self_dentry: &Dentry, time: Duration) {
        self.set_ctime_impl(self_dentry, time)
    }

    fn get_xattr(&self, name: XattrName, value_writer: &mut VmWriter) -> Result<usize> {
        self.get_xattr_impl(name, value_writer)
    }

    fn set_xattr(
        &self,
        self_dentry: &Dentry,
        name: XattrName,
        value_reader: &mut VmReader,
        flags: XattrSetFlags,
    ) -> Result<()> {
        self.set_xattr_impl(self_dentry, name, value_reader, flags)
    }

    fn list_xattr(&self, namespace: XattrNamespace, list_writer: &mut VmWriter) -> Result<usize> {
        self.list_xattr_impl(namespace, list_writer)
    }

    fn remove_xattr(&self, self_dentry: &Dentry, name: XattrName) -> Result<()> {
        self.remove_xattr_impl(self_dentry, name)
    }

    fn create(
        &self,
        self_dentry: &Dentry,
        name: &str,
        type_: InodeType,
        mode: InodeMode,
    ) -> Result<Arc<dyn Inode>> {
        self.create_impl(self_dentry, name, CreateOp::general(type_, mode))
    }

    fn create_symlink(
        &self,
        self_dentry: &Dentry,
        name: &str,
        target: &str,
        mode: InodeMode,
    ) -> Result<Arc<dyn Inode>> {
        self.create_impl(self_dentry, name, CreateOp::symlink(target, mode))
    }

    fn mknod(
        &self,
        self_dentry: &Dentry,
        name: &str,
        mode: InodeMode,
        type_: MknodType,
    ) -> Result<Arc<dyn Inode>> {
        self.create_impl(self_dentry, name, CreateOp::mknod(&type_, mode)?)
    }

    fn link(&self, self_dentry: &Dentry, old_dentry: &Dentry, name: &str) -> Result<()> {
        self.link_impl(self_dentry, old_dentry, name)
    }

    fn unlink(&self, child_dentry: &Dentry) -> Result<()> {
        self.unlink_impl(child_dentry)
    }

    fn rmdir(&self, child_dentry: &Dentry) -> Result<()> {
        self.rmdir_impl(child_dentry)
    }

    fn rename(
        &self,
        old_child_dentry: &Dentry,
        new_dir_dentry: &Dentry,
        new_name: &str,
        replaced_inode: Option<&Arc<dyn Inode>>,
        mode: RenameMode,
    ) -> Result<()> {
        self.rename_impl(
            old_child_dentry,
            new_dir_dentry,
            new_name,
            replaced_inode,
            mode,
        )
    }
}
