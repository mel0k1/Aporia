#![expect(unused_variables)]

use alloc::boxed::ThinBox;
use core::time::Duration;

use device_id::DeviceId;
use ostd::task::{CurrentTask, Task};
use spin::Once;

use super::{
    file_system::FileSystem,
    xattr::{XattrName, XattrNamespace, XattrSetFlags},
};
use crate::{
    device::{Device, DeviceType},
    fs::{
        file::{
            AccessMode, InodeMode, InodeType, PerOpenFileOps, Permission, StatusFlags, SyncMode,
        },
        utils::DirentVisitor,
        vfs::path::{Dentry, Path},
    },
    prelude::*,
    process::{
        Gid, Uid,
        credentials::capabilities::CapSet,
        posix_thread::{AsPosixThread, PosixThread},
    },
    security::lsm::hooks as lsm_hooks,
    time::clocks::RealTimeCoarseClock,
    vm::page_cache::Vmo,
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Metadata {
    pub ino: u64,

    pub size: usize,

    pub optimal_block_size: usize,

    pub nr_sectors_allocated: usize,

    pub last_access_at: Duration,

    pub last_modify_at: Duration,

    pub last_meta_change_at: Duration,

    pub type_: InodeType,

    pub mode: InodeMode,

    pub nr_hard_links: usize,

    pub uid: Uid,

    pub gid: Gid,

    pub container_dev_id: DeviceId,

    pub self_dev_id: Option<DeviceId>,

    pub birth_at: Option<Duration>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HardLinkability {
    Linkable,
    Unlinkable,
}

impl Metadata {
    pub(crate) fn new_dir(
        ino: u64,
        mode: InodeMode,
        blk_size: usize,
        container_dev_id: DeviceId,
    ) -> Self {
        let now = RealTimeCoarseClock::get().read_time();
        Self {
            ino,
            size: 2,
            optimal_block_size: blk_size,
            nr_sectors_allocated: 0,
            last_access_at: now,
            last_modify_at: now,
            last_meta_change_at: now,
            type_: InodeType::Dir,
            mode,
            nr_hard_links: 2,
            uid: Uid::new_root(),
            gid: Gid::new_root(),
            container_dev_id,
            self_dev_id: None,
            birth_at: None,
        }
    }

    pub(crate) fn new_file(
        ino: u64,
        mode: InodeMode,
        blk_size: usize,
        container_dev_id: DeviceId,
    ) -> Self {
        let now = RealTimeCoarseClock::get().read_time();
        Self {
            ino,
            size: 0,
            optimal_block_size: blk_size,
            nr_sectors_allocated: 0,
            last_access_at: now,
            last_modify_at: now,
            last_meta_change_at: now,
            type_: InodeType::File,
            mode,
            nr_hard_links: 1,
            uid: Uid::new_root(),
            gid: Gid::new_root(),
            container_dev_id,
            self_dev_id: None,
            birth_at: None,
        }
    }

    pub(crate) fn new_symlink(
        ino: u64,
        mode: InodeMode,
        blk_size: usize,
        container_dev_id: DeviceId,
    ) -> Self {
        let now = RealTimeCoarseClock::get().read_time();
        Self {
            ino,
            size: 0,
            optimal_block_size: blk_size,
            nr_sectors_allocated: 0,
            last_access_at: now,
            last_modify_at: now,
            last_meta_change_at: now,
            type_: InodeType::SymLink,
            mode,
            nr_hard_links: 1,
            uid: Uid::new_root(),
            gid: Gid::new_root(),
            container_dev_id,
            self_dev_id: None,
            birth_at: None,
        }
    }

    pub(crate) fn new_device(
        ino: u64,
        mode: InodeMode,
        blk_size: usize,
        device: &dyn Device,
        container_dev_id: DeviceId,
    ) -> Self {
        let now = RealTimeCoarseClock::get().read_time();
        Self {
            ino,
            size: 0,
            optimal_block_size: blk_size,
            nr_sectors_allocated: 0,
            last_access_at: now,
            last_modify_at: now,
            last_meta_change_at: now,
            type_: InodeType::from(device.type_()),
            mode,
            nr_hard_links: 1,
            uid: Uid::new_root(),
            gid: Gid::new_root(),
            container_dev_id,
            self_dev_id: Some(device.id()),
            birth_at: None,
        }
    }
}

pub(crate) enum MknodType {
    NamedPipe,
    CharDevice(u64),
    BlockDevice(u64),
}

impl MknodType {
    pub(crate) fn device_type(&self) -> Option<DeviceType> {
        match self {
            MknodType::NamedPipe => None,
            MknodType::CharDevice(_) => Some(DeviceType::Char),
            MknodType::BlockDevice(_) => Some(DeviceType::Block),
        }
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub(crate) struct RevalidationPolicy: u8 {
        const REVALIDATE_EXISTS = 1 << 0;

        const REVALIDATE_ABSENT = 1 << 1;
    }
}

pub trait FileOps {
    fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
    ) -> Result<usize>;

    fn write_at(
        &self,
        offset: usize,
        reader: &mut VmReader,
        status_flags: StatusFlags,
    ) -> Result<usize>;

    fn readdir_at(&self, _offset: usize, _visitor: &mut dyn DirentVisitor) -> Result<usize> {
        Err(errno!(ENOTDIR, "readdir is not supported"))
    }
}

pub(crate) trait Inode: Any + FileOps + Send + Sync {
    fn size(&self) -> usize;

    fn resize(&self, self_dentry: &Dentry, new_size: usize) -> Result<()>;

    fn metadata(&self) -> Result<Metadata>;

    fn ino(&self) -> u64;

    fn type_(&self) -> InodeType;

    fn mode(&self) -> Result<InodeMode>;

    fn set_mode(&self, self_dentry: &Dentry, mode: InodeMode) -> Result<()>;

    fn owner(&self) -> Result<Uid>;

    fn set_owner(&self, self_dentry: &Dentry, uid: Uid) -> Result<()>;

    fn group(&self) -> Result<Gid>;

    fn set_group(&self, self_dentry: &Dentry, gid: Gid) -> Result<()>;

    fn atime(&self) -> Duration;

    fn set_atime(&self, self_dentry: &Dentry, time: Duration);

    fn mtime(&self) -> Duration;

    fn set_mtime(&self, self_dentry: &Dentry, time: Duration);

    fn ctime(&self) -> Duration;

    fn set_ctime(&self, self_dentry: &Dentry, time: Duration);

    fn page_cache(&self) -> Option<Arc<Vmo>> {
        None
    }

    fn create(
        &self,
        self_dentry: &Dentry,
        name: &str,
        type_: InodeType,
        mode: InodeMode,
    ) -> Result<Arc<dyn Inode>> {
        Err(errno!(ENOTDIR))
    }

    fn create_symlink(
        &self,
        self_dentry: &Dentry,
        name: &str,
        target: &str,
        mode: InodeMode,
    ) -> Result<Arc<dyn Inode>> {
        Err(errno!(EOPNOTSUPP))
    }

    fn create_tmpfile(
        &self,
        self_dentry: &Dentry,
        mode: InodeMode,
        hard_linkability: HardLinkability,
    ) -> Result<Arc<dyn Inode>> {
        Err(errno!(EOPNOTSUPP))
    }

    fn mknod(
        &self,
        self_dentry: &Dentry,
        name: &str,
        mode: InodeMode,
        type_: MknodType,
    ) -> Result<Arc<dyn Inode>> {
        Err(errno!(ENOTDIR))
    }

    fn open(
        &self,
        self_dentry: &Dentry,
        access_mode: AccessMode,
        status_flags: StatusFlags,
    ) -> Option<Result<Box<dyn PerOpenFileOps>>> {
        None
    }

    fn link(&self, self_dentry: &Dentry, old_dentry: &Dentry, name: &str) -> Result<()> {
        Err(errno!(ENOTDIR))
    }

    fn unlink(&self, child_dentry: &Dentry) -> Result<()> {
        Err(errno!(ENOTDIR))
    }

    fn rmdir(&self, child_dentry: &Dentry) -> Result<()> {
        Err(errno!(ENOTDIR))
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        Err(errno!(ENOTDIR))
    }

    fn rename(
        &self,
        old_child_dentry: &Dentry,
        new_dir_dentry: &Dentry,
        new_name: &str,
        replaced_inode: Option<&Arc<dyn Inode>>,
        mode: RenameMode,
    ) -> Result<()> {
        Err(errno!(ENOTDIR))
    }

    fn read_link(&self) -> Result<SymbolicLink> {
        Err(errno!(EISDIR))
    }

    fn sync(&self, _mode: SyncMode) -> Result<()> {
        Ok(())
    }

    fn fallocate(&self, mode: FallocMode, offset: usize, len: usize) -> Result<()> {
        Err(errno!(EOPNOTSUPP))
    }

    fn fs(&self) -> Arc<dyn FileSystem>;

    fn revalidation_policy(&self) -> RevalidationPolicy {
        RevalidationPolicy::empty()
    }

    fn revalidate_exists(&self, _name: &str, _child: &dyn Inode) -> bool {
        true
    }

    fn revalidate_absent(&self, _name: &str) -> bool {
        true
    }

    fn seek_end(&self) -> Option<usize> {
        if self.type_() == InodeType::File {
            Some(self.size())
        } else {
            None
        }
    }

    fn extension(&self) -> &Extension;

    fn set_xattr(
        &self,
        self_dentry: &Dentry,
        name: XattrName,
        value_reader: &mut VmReader,
        flags: XattrSetFlags,
    ) -> Result<()> {
        Err(errno!(EOPNOTSUPP))
    }

    fn get_xattr(&self, name: XattrName, value_writer: &mut VmWriter) -> Result<usize> {
        Err(errno!(EOPNOTSUPP))
    }

    fn list_xattr(&self, namespace: XattrNamespace, list_writer: &mut VmWriter) -> Result<usize> {
        Err(errno!(EOPNOTSUPP))
    }

    fn remove_xattr(&self, self_dentry: &Dentry, _name: XattrName) -> Result<()> {
        Err(errno!(EOPNOTSUPP))
    }

    fn check_permission(&self, mut perm: Permission) -> Result<()> {
        let Some(task) = Task::current() else {
            return Ok(());
        };
        let Some(posix_thread) = task.as_posix_thread() else {
            return Ok(());
        };

        let creds = posix_thread.credentials();
        let metadata = self.metadata()?;
        let mode = metadata.mode;

        if has_dac_override_capability(&task, posix_thread) {
            if metadata.type_ == InodeType::Dir {
                perm -= Permission::MAY_READ | Permission::MAY_WRITE | Permission::MAY_EXEC;
            } else {
                perm -= Permission::MAY_READ | Permission::MAY_WRITE;

                if perm.may_exec() {
                    if mode.is_owner_executable()
                        || mode.is_group_executable()
                        || mode.is_other_executable()
                    {
                        perm -= Permission::MAY_EXEC;
                    } else {
                        return Err(errno!(
                            EACCES,
                            "root execute permission denied: no execute bits set"
                        ));
                    }
                }
            }
        }

        if metadata.uid == creds.fsuid() {
            if (perm.may_read() && !mode.is_owner_readable())
                || (perm.may_write() && !mode.is_owner_writable())
                || (perm.may_exec() && !mode.is_owner_executable())
            {
                return Err(errno!(EACCES, "owner permission check failed"));
            }
        } else if creds.has_group_for_fs_access(metadata.gid) {
            if (perm.may_read() && !mode.is_group_readable())
                || (perm.may_write() && !mode.is_group_writable())
                || (perm.may_exec() && !mode.is_group_executable())
            {
                return Err(errno!(EACCES, "group permission check failed"));
            }
        } else if (perm.may_read() && !mode.is_other_readable())
            || (perm.may_write() && !mode.is_other_writable())
            || (perm.may_exec() && !mode.is_other_executable())
        {
            return Err(errno!(EACCES, "other permission check failed"));
        }

        Ok(())
    }
}

fn has_dac_override_capability(task: &CurrentTask, posix_thread: &PosixThread) -> bool {
    let thread_local = task.as_thread_local().unwrap();
    let user_ns = thread_local.borrow_user_ns();
    lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
        user_ns.as_ref(),
        posix_thread,
        CapSet::DAC_OVERRIDE,
    ))
    .is_ok()
}

impl dyn Inode {
    pub(crate) fn downcast_ref<T: Inode>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref::<T>()
    }

    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(crate) fn read_bytes_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        let mut writer = VmWriter::from(buf).to_fallible();
        self.read_at(offset, &mut writer, StatusFlags::empty())
    }

    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(crate) fn write_bytes_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        let mut reader = VmReader::from(buf).to_fallible();
        self.write_at(offset, &mut reader, StatusFlags::empty())
    }
}

impl Debug for dyn Inode {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("Inode")
            .field("metadata", &self.metadata())
            .field("fs", &self.fs())
            .finish()
    }
}

#[derive(Debug)]
pub(crate) struct Extension {
    group1: Once<ThinBox<dyn Any + Send + Sync>>,
    group2: Once<ThinBox<dyn Any + Send + Sync>>,
    group3: Once<ThinBox<dyn Any + Send + Sync>>,
}

impl Extension {
    pub(crate) fn new() -> Self {
        Self {
            group1: Once::new(),
            group2: Once::new(),
            group3: Once::new(),
        }
    }

    pub(crate) fn group1(&self) -> &Once<ThinBox<dyn Any + Send + Sync>> {
        &self.group1
    }

    pub(crate) fn group2(&self) -> &Once<ThinBox<dyn Any + Send + Sync>> {
        &self.group2
    }

    pub(crate) fn group3(&self) -> &Once<ThinBox<dyn Any + Send + Sync>> {
        &self.group3
    }
}

#[derive(Clone, Debug)]
pub(crate) enum SymbolicLink {
    Plain(String),
    Path(Path),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FallocMode {
    Allocate,
    AllocateKeepSize,
    AllocateUnshareRange,
    PunchHoleKeepSize,
    ZeroRange,
    ZeroRangeKeepSize,
    CollapseRange,
    InsertRange,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RenameMode {
    Replace,
    NoReplace,
    Exchange,
}
