macro_rules! __log_prefix {
    () => {
        "rootfs: "
    };
}

use core::{
    str::FromStr,
    sync::atomic::{AtomicBool, Ordering},
};

use aster_cmdline::parse::ParamStorage;
use spin::once::Once;

use super::{
    ext2,
    vfs::{
        file_system::FsFlags,
        path::{FsPath, Mount, MountNamespace, Path, PathResolver, PerMountFlags},
        registry::{DynFsType, FsAndRoot, FsCreationCtx},
    },
};
use crate::prelude::*;

pub(crate) static SUPPORTED_ROOTFS_TYPES: &[&dyn DynFsType] = &[&ext2::EXT2_TYPE];

pub(crate) fn switch_to_rootfs(path_resolver: &mut PathResolver) -> Result<()> {
    let root = ROOT_PATH
        .get()
        .ok_or_else(|| errno!(EINVAL, "the `root` parameter was not provided"))?;
    let (fs_flags, mount_flags) = rootfs_flags();
    let fs_and_root = open_rootfs(root, fs_flags)?;
    let rootfs_mount = Mount::new_detached(
        fs_and_root,
        mount_flags,
        Arc::downgrade(MountNamespace::get_init_singleton()),
        Some(root.to_string()),
    )?;
    path_resolver.switch_root_for_boot(rootfs_mount);
    println!("[kernel] rootfs is ready (mounted from {})", root);
    Ok(())
}

pub(crate) fn find_init(path_resolver: &PathResolver) -> Result<Option<(Path, &'static str)>> {
    let Some(init_path) = INIT_PATH.get().map(String::as_str) else {
        return Ok(None);
    };

    let path = path_resolver.lookup(&FsPath::try_from(init_path)?)?;
    Ok(Some((path, init_path)))
}

fn open_rootfs(root: &str, fs_flags: FsFlags) -> Result<FsAndRoot> {
    let device_name = root
        .strip_prefix("/dev/")
        .ok_or_else(|| errno!(EINVAL, "root must name a /dev block device"))?;
    let device = aster_block::lookup_by_name(device_name)
        .ok_or_else(|| errno!(ENODEV, "root block device not found"))?;
    open_rootfs_from_candidates(device, rootfs_types(), fs_flags)
}

fn open_rootfs_from_candidates(
    device: Arc<dyn aster_block::BlockDevice>,
    rootfs_types: &[&dyn DynFsType],
    fs_flags: FsFlags,
) -> Result<FsAndRoot> {
    let mut fs_creation_ctx = FsCreationCtx::from_block_device(device, fs_flags, None);
    for rootfs_type in rootfs_types {
        match rootfs_type.get_or_create(&mut fs_creation_ctx) {
            Ok(fs_and_root) => return Ok(fs_and_root),
            Err(err) if err.error() == Errno::EINVAL => continue,
            Err(err) => return Err(err),
        }
    }

    Err(errno!(ENODEV, "no root filesystem type could mount root"))
}

fn rootfs_flags() -> (FsFlags, PerMountFlags) {
    let mut fs_flags = FsFlags::empty();
    let mut mount_flags = PerMountFlags::default();
    if ROOT_MOUNT_READ_ONLY.load(Ordering::Relaxed) {
        fs_flags.insert(FsFlags::RDONLY);
        mount_flags.insert(PerMountFlags::RDONLY);
    }
    (fs_flags, mount_flags)
}

fn rootfs_types() -> &'static [&'static dyn DynFsType] {
    ROOTFS_TYPES
        .get()
        .map(RootFsTypes::as_slice)
        .unwrap_or(SUPPORTED_ROOTFS_TYPES)
}

static ROOT_PATH: Once<String> = Once::new();
aster_cmdline::define_kv_param!("root", ROOT_PATH);

static INIT_PATH: Once<String> = Once::new();
aster_cmdline::define_kv_param!("init", INIT_PATH);

struct RootFsTypes(Vec<&'static dyn DynFsType>);

impl RootFsTypes {
    fn as_slice(&self) -> &[&'static dyn DynFsType] {
        self.0.as_slice()
    }
}

impl FromStr for RootFsTypes {
    type Err = core::convert::Infallible;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let candidates = value
            .split(',')
            .filter(|type_name| !type_name.is_empty())
            .filter_map(|type_name| {
                SUPPORTED_ROOTFS_TYPES
                    .iter()
                    .copied()
                    .find(|fs_type| fs_type.name() == type_name)
            })
            .collect();

        Ok(Self(candidates))
    }
}

static ROOTFS_TYPES: Once<RootFsTypes> = Once::new();
aster_cmdline::define_kv_param!("rootfstype", ROOTFS_TYPES);

static ROOT_MOUNT_READ_ONLY: AtomicBool = AtomicBool::new(true);

struct SetRootMountReadOnly;
struct SetRootMountReadWrite;

impl ParamStorage for SetRootMountReadOnly {
    type Value = bool;

    fn store_param(&self, value: Self::Value) {
        if value {
            ROOT_MOUNT_READ_ONLY.store(true, Ordering::Relaxed);
        }
    }
}

impl ParamStorage for SetRootMountReadWrite {
    type Value = bool;

    fn store_param(&self, value: Self::Value) {
        if value {
            ROOT_MOUNT_READ_ONLY.store(false, Ordering::Relaxed);
        }
    }
}

static RO_PARAM: SetRootMountReadOnly = SetRootMountReadOnly;
static RW_PARAM: SetRootMountReadWrite = SetRootMountReadWrite;
aster_cmdline::define_flag_param!("ro", RO_PARAM);
aster_cmdline::define_flag_param!("rw", RW_PARAM);
