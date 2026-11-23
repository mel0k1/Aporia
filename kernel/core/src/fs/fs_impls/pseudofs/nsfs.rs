use alloc::format;
use core::time::Duration;

use inherit_methods_macro::inherit_methods;
use ostd::task::Task;
use spin::Once;

use crate::{
    dispatch_ioctl,
    events::IoEvents,
    fs::{
        file::{
            AccessMode, InodeHandle, InodeMode, InodeType, PerOpenFileOps, StatusFlags,
            file_table::{FdFlags, FileDesc, FileTableInsert},
            mkmod,
        },
        pseudofs::{NaivePseudoFs, PseudoInode, PseudoInodeType},
        vfs::{
            file_system::FileSystem,
            inode::{Extension, FileOps, Inode, Metadata},
            path::{Dentry, Mount, Path},
        },
    },
    prelude::*,
    process::{
        CloneFlags, Gid, Uid, UserNamespace,
        signal::{PollHandle, Pollable},
    },
    util::ioctl::RawIoctl,
};

pub(super) fn init() {
    let _ = NsFs::mount_node();
}

struct NsFs {
    _private: (),
}

impl NsFs {
    pub(self) fn singleton() -> &'static Arc<NaivePseudoFs> {
        static NSFS: Once<Arc<NaivePseudoFs>> = Once::new();
        NaivePseudoFs::singleton(&NSFS, "nsfs", NSFS_MAGIC)
    }

    pub(self) fn new_path<T: NsCommonOps>(ino: u64, ns: Arc<T>) -> Path {
        let ns_inode = {
            let fs = Arc::downgrade(Self::singleton());
            Arc::new(NsInode::new(ino, Uid::new_root(), Gid::new_root(), ns, fs))
        };

        Path::new_pseudo(Self::mount_node().clone(), ns_inode, |inode| {
            let inode = inode.downcast_ref::<NsInode<T>>().unwrap();
            inode.name().to_string()
        })
    }

    pub(self) fn mount_node() -> &'static Arc<Mount> {
        static NSFS_MOUNT: Once<Arc<Mount>> = Once::new();
        NSFS_MOUNT.call_once(|| Mount::new_pseudo(Self::singleton().clone()).unwrap())
    }
}

pub(in crate::fs) struct NsInode<T: NsCommonOps> {
    common: PseudoInode,
    ns: Arc<T>,
    name: String,
}

impl<T: NsCommonOps> NsInode<T> {
    fn new(ino: u64, uid: Uid, gid: Gid, ns: Arc<T>, fs: Weak<NaivePseudoFs>) -> Self {
        let mode = mkmod!(a+r);
        let fs = fs.upgrade().unwrap();
        let common = PseudoInode::new(
            ino,
            PseudoInodeType::Ns,
            mode,
            uid,
            gid,
            Arc::downgrade(&fs),
            fs.container_dev_id(),
        );
        let name = format!("{}:[{}]", T::NAME, ino);

        Self { common, ns, name }
    }

    fn name(&self) -> &str {
        &self.name
    }

    pub(in crate::fs) fn ns(&self) -> &Arc<T> {
        &self.ns
    }
}

#[inherit_methods(from = "self.common")]
impl<T: NsCommonOps> Inode for NsInode<T> {
    fn size(&self) -> usize;
    fn resize(&self, self_dentry: &Dentry, _new_size: usize) -> Result<()>;
    fn metadata(&self) -> Result<Metadata>;
    fn extension(&self) -> &Extension;
    fn ino(&self) -> u64;
    fn type_(&self) -> InodeType;
    fn mode(&self) -> Result<InodeMode>;
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
    fn fs(&self) -> Arc<dyn FileSystem>;

    fn open(
        &self,
        _self_dentry: &Dentry,
        access_mode: AccessMode,
        _status_flags: StatusFlags,
    ) -> Option<Result<Box<dyn PerOpenFileOps>>> {
        if access_mode.is_writable() {
            return Some(Err(errno!(EPERM, "ns files cannot be opened as writable",)));
        }

        let ns_file = NsFile {
            ns: self.ns.clone(),
        };
        Some(Ok(Box::new(ns_file) as Box<dyn PerOpenFileOps>))
    }

    fn set_mode(&self, _self_dentry: &Dentry, _mode: InodeMode) -> Result<()> {
        Err(errno!(EPERM, "the mode of ns inodes cannot be changed"))
    }
}

#[inherit_methods(from = "self.common")]
impl<T: NsCommonOps> FileOps for NsInode<T> {
    fn read_at(
        &self,
        _offset: usize,
        _writer: &mut VmWriter,
        _status: StatusFlags,
    ) -> Result<usize>;
    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status: StatusFlags,
    ) -> Result<usize>;
}

pub(crate) struct NsFile<T: NsCommonOps> {
    ns: Arc<T>,
}

impl<T: NsCommonOps> NsFile<T> {
    pub(crate) fn ns(&self) -> &Arc<T> {
        &self.ns
    }
}

impl<T: NsCommonOps> PerOpenFileOps for NsFile<T> {
    fn check_seekable(&self) -> Result<()> {
        Err(errno!(ESPIPE, "ns files are not seekable"))
    }

    fn is_offset_aware(&self) -> bool {
        false
    }

    fn check_positional_io(&self) -> Result<()> {
        Ok(())
    }

    fn ioctl(&self, _path: &Path, raw_ioctl: RawIoctl) -> Result<i32> {
        use ioctl_defs::*;

        dispatch_ioctl!(match raw_ioctl {
            _cmd @ GetUserNs => {
                let user_ns = self.ns.owner_user_ns().ok_or_else(|| {
                    errno!(EPERM, "the namespace does not have an owner user namespace",)
                })?;

                let current = current!();
                let current_user_ns = current.user_ns().lock();
                if !current_user_ns.is_same_or_ancestor_of(user_ns) {
                    return Err(errno!(
                        EPERM,
                        "the current namespace is not an ancestor of the owner user namespace"
                    ));
                }

                open_ns_as_file(user_ns).map(|fd| fd.into())
            }
            _cmd @ GetParent => {
                let parent = self.ns.parent()?;
                open_ns_as_file(parent).map(|fd| fd.into())
            }
            _cmd @ GetType => {
                let clone_flags = CloneFlags::from(T::TYPE);
                Ok(clone_flags.bits().cast_signed())
            }
            cmd @ GetOwnerUid => {
                let ns = self.ns.as_ref() as &dyn Any;
                let user_ns = ns.downcast_ref::<UserNamespace>().ok_or_else(|| {
                    errno!(
                        EINVAL,
                        "the ns file does not correspond to a user namespace",
                    )
                })?;
                let uid = user_ns.owner_uid()?;
                cmd.write(&uid.into())?;
                Ok(0)
            }
            _ => Err(errno!(ENOTTY, "the ioctl command is unknown")),
        })
    }
}

impl<T: NsCommonOps> Pollable for NsFile<T> {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        (IoEvents::IN | IoEvents::OUT | IoEvents::RDNORM) & mask
    }
}

impl<T: NsCommonOps> FileOps for NsFile<T> {
    fn read_at(
        &self,
        _offset: usize,
        _writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        Err(errno!(
            EINVAL,
            "ns files cannot be read at a specific offset"
        ))
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        Err(errno!(
            EINVAL,
            "ns files cannot be written at a specific offset"
        ))
    }
}

fn open_ns_as_file<T: NsCommonOps>(ns: &Arc<T>) -> Result<FileDesc> {
    let path = ns.get_path();
    let inode_handle = InodeHandle::new(path.clone(), AccessMode::O_RDONLY, StatusFlags::empty())?;

    let current_task = Task::current().unwrap();
    let thread_local = current_task.as_thread_local().unwrap();
    let file_table = thread_local.borrow_file_table();
    file_table.lock_and_insert(Arc::new(inode_handle), FdFlags::CLOEXEC)
}

pub(crate) trait NsCommonOps: Any + Send + Sync + 'static + Sized {
    const NAME: &str = Self::TYPE.as_str();

    const TYPE: NsType;

    fn owner_user_ns(&self) -> Option<&Arc<UserNamespace>>;

    fn parent(&self) -> Result<&Arc<Self>>;

    fn stashed_dentry(&self) -> &StashedDentry;

    fn get_path(self: &Arc<Self>) -> Path {
        self.stashed_dentry().get_path(self)
    }
}

pub(crate) struct StashedDentry {
    ino: u64,
    dentry: Mutex<Weak<Dentry>>,
}

impl PartialEq for StashedDentry {
    fn eq(&self, other: &Self) -> bool {
        self.ino == other.ino
    }
}

impl Eq for StashedDentry {}

impl PartialOrd for StashedDentry {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for StashedDentry {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.ino.cmp(&other.ino)
    }
}

impl StashedDentry {
    pub(crate) fn new() -> Self {
        let ino = NsFs::singleton().alloc_id();
        Self {
            ino,
            dentry: Mutex::new(Weak::new()),
        }
    }

    pub(self) fn get_path<T: NsCommonOps>(&self, ns: &Arc<T>) -> Path {
        let mut dentry = self.dentry.lock();
        if let Some(dentry) = dentry.upgrade() {
            return Path::new(NsFs::mount_node().clone(), dentry);
        }

        let path = NsFs::new_path(self.ino, ns.clone());
        *dentry = Arc::downgrade(path.dentry());

        path
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum NsType {
    Cgroup,
    Ipc,
    Mnt,
    #[expect(unused)]
    Net,
    #[expect(unused)]
    Pid,
    #[expect(unused)]
    Time,
    User,
    Uts,
}

impl NsType {
    const fn as_str(&self) -> &'static str {
        match self {
            NsType::Cgroup => "cgroup",
            NsType::Ipc => "ipc",
            NsType::Mnt => "mnt",
            NsType::Net => "net",
            NsType::Pid => "pid",
            NsType::Time => "time",
            NsType::User => "user",
            NsType::Uts => "uts",
        }
    }
}

impl From<NsType> for CloneFlags {
    fn from(value: NsType) -> Self {
        match value {
            NsType::Cgroup => CloneFlags::CLONE_NEWCGROUP,
            NsType::Ipc => CloneFlags::CLONE_NEWIPC,
            NsType::Mnt => CloneFlags::CLONE_NEWNS,
            NsType::Net => CloneFlags::CLONE_NEWNET,
            NsType::Pid => CloneFlags::CLONE_NEWPID,
            NsType::Time => CloneFlags::CLONE_NEWTIME,
            NsType::User => CloneFlags::CLONE_NEWUSER,
            NsType::Uts => CloneFlags::CLONE_NEWUTS,
        }
    }
}

const NSFS_MAGIC: u64 = 0x6e736673;

mod ioctl_defs {
    use crate::{
        ioc,
        util::ioctl::{InData, NoData, OutData},
    };

    pub(crate) type GetUserNs       = ioc!(NS_GET_USERNS,     0xb701, NoData);
    pub(crate) type GetParent       = ioc!(NS_GET_PARENT,     0xb702, NoData);
    pub(crate) type GetType         = ioc!(NS_GET_NSTYPE,     0xb703, NoData);
    pub(crate) type GetOwnerUid     = ioc!(NS_GET_OWNER_UID,  0xb704, OutData<u32>);

    #[expect(unused)]
    pub(crate) type GetMntNsId      = ioc!(NS_GET_MNTNS_ID,        0xb7, 0x5, OutData<u64>);
    #[expect(unused)]
    pub(crate) type GetTidFromPidNs = ioc!(NS_GET_PID_FROM_PIDNS,  0xb7, 0x6, InData<i32>);
    #[expect(unused)]
    pub(crate) type GetPidFromPidNs = ioc!(NS_GET_TGID_FROM_PIDNS, 0xb7, 0x7, InData<i32>);
    #[expect(unused)]
    pub(crate) type GetTidInPidNs   = ioc!(NS_GET_PID_IN_PIDNS,    0xb7, 0x8, InData<i32>);
    #[expect(unused)]
    pub(crate) type GetPidInPidNs   = ioc!(NS_GET_TGID_IN_PIDNS,   0xb7, 0x9, InData<i32>);
}
