use alloc::collections::btree_map::Entry;

use aster_block::BlockDevice;
use aster_systree::{
    AttrLessBranchNodeFields, SysNode, SysObj, SysPerms, SysStr, inherit_sys_branch_node,
};
use spin::Once;

use crate::{
    fs::{
        fs_impls::sysfs,
        vfs::{
            file_system::{FileSystem, FsFlags},
            path::{AT_FDCWD, Dentry, EmptyPathStr, FsPath},
        },
    },
    prelude::*,
};

pub(crate) trait FsType: Send + Sync + 'static {
    type Key: Eq + Ord + Clone + Send + Sync + 'static;

    fn name(&self) -> &'static str;

    fn properties(&self) -> FsProperties;

    fn create(&self, fs_creation_ctx: &mut FsCreationCtx) -> Result<Arc<dyn FileSystem>>;

    fn obtain_key_and_cache(
        &self,
        _fs_creation_ctx: &mut FsCreationCtx,
    ) -> Option<(Self::Key, &FsCache<Self::Key>)> {
        None
    }

    fn sysnode(&self) -> Option<Arc<dyn SysNode>>;
}

pub(crate) trait DynFsType: Send + Sync + 'static {
    fn name(&self) -> &'static str;

    fn properties(&self) -> FsProperties;

    fn get_or_create(&self, fs_creation_ctx: &mut FsCreationCtx) -> Result<FsAndRoot>;

    fn sysnode(&self) -> Option<Arc<dyn SysNode>>;
}

impl<T: FsType> DynFsType for T {
    fn name(&self) -> &'static str {
        <T as FsType>::name(self)
    }

    fn properties(&self) -> FsProperties {
        <T as FsType>::properties(self)
    }

    fn sysnode(&self) -> Option<Arc<dyn SysNode>> {
        <T as FsType>::sysnode(self)
    }

    fn get_or_create(&self, fs_creation_ctx: &mut FsCreationCtx) -> Result<FsAndRoot> {
        if let Some((key, cache)) = self.obtain_key_and_cache(fs_creation_ctx) {
            cache.get_or_create(key, fs_creation_ctx.flags(), || {
                self.create(fs_creation_ctx)
            })
        } else {
            let fs = self.create(fs_creation_ctx)?;
            Ok(FsAndRoot::new(fs))
        }
    }
}

pub(crate) struct FsCreationCtx<'a> {
    source: Option<&'a str>,
    flags: FsFlags,
    args: Option<&'a str>,
    block_device: BlockDeviceResolution<'a>,
}

enum BlockDeviceResolution<'a> {
    Pending(&'a Context<'a>),
    Resolved(Arc<dyn BlockDevice>),
}

impl<'a> FsCreationCtx<'a> {
    pub(crate) fn new(
        source: Option<&'a str>,
        flags: FsFlags,
        args: Option<&'a str>,
        task_ctx: &'a Context<'a>,
    ) -> Self {
        Self {
            source,
            flags,
            args,
            block_device: BlockDeviceResolution::Pending(task_ctx),
        }
    }

    pub(in crate::fs) fn from_block_device(
        block_device: Arc<dyn BlockDevice>,
        flags: FsFlags,
        args: Option<&'a str>,
    ) -> Self {
        Self {
            source: None,
            flags,
            args,
            block_device: BlockDeviceResolution::Resolved(block_device),
        }
    }

    pub(in crate::fs) fn source(&self) -> Option<&str> {
        self.source
    }

    pub(in crate::fs) fn flags(&self) -> FsFlags {
        self.flags
    }

    pub(in crate::fs) fn args(&self) -> Option<&str> {
        self.args
    }

    pub(in crate::fs) fn ctx(&self) -> Option<&'a Context<'a>> {
        match &self.block_device {
            BlockDeviceResolution::Pending(task_ctx) => Some(*task_ctx),
            BlockDeviceResolution::Resolved(_) => None,
        }
    }

    pub(in crate::fs) fn resolve_block_device(&mut self) -> Result<&Arc<dyn BlockDevice>> {
        if self.block_device().is_none() {
            self.do_resolve_block_device()?;
        }

        Ok(self.block_device().unwrap())
    }

    fn block_device(&self) -> Option<&Arc<dyn BlockDevice>> {
        match &self.block_device {
            BlockDeviceResolution::Pending(_) => None,
            BlockDeviceResolution::Resolved(block_device) => Some(block_device),
        }
    }

    fn do_resolve_block_device(&mut self) -> Result<()> {
        let task_ctx = match &self.block_device {
            BlockDeviceResolution::Pending(task_ctx) => *task_ctx,
            BlockDeviceResolution::Resolved(_) => return Ok(()),
        };

        let source = self
            .source()
            .ok_or_else(|| errno!(EINVAL, "the source is not specified"))?;
        let fs_path = FsPath::from_fd_at(AT_FDCWD, source, EmptyPathStr::Reject)?;
        let path = task_ctx
            .thread_local
            .borrow_fs()
            .resolver()
            .read()
            .lookup(&fs_path)?;

        if !path.type_().is_device() {
            return Err(errno!(ENODEV, "the path is not a device file"));
        }
        let id = path.metadata()?.self_dev_id;

        let block_device = id
            .and_then(aster_block::lookup)
            .ok_or_else(|| errno!(ENODEV, "the device is not found"))?;
        self.block_device = BlockDeviceResolution::Resolved(block_device);

        Ok(())
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub(crate) struct FsProperties: u32 {
        const NEED_DISK = 1 << 1;
    }
}

pub(crate) fn register(new_type: &'static dyn DynFsType) -> Result<()> {
    FS_REGISTRY.get().unwrap().register(new_type)
}

pub(crate) fn look_up(name: &str) -> Option<&'static dyn DynFsType> {
    FS_REGISTRY
        .get()
        .unwrap()
        .fs_table
        .lock()
        .get(name)
        .cloned()
}

pub(crate) fn with_iter<F, R>(f: F) -> R
where
    F: FnOnce(&mut dyn Iterator<Item = (&str, &dyn DynFsType)>) -> R,
{
    let guard = FS_REGISTRY.get().unwrap().fs_table.lock();

    let mut iter = guard.iter().map(|(name, fs_type)| (*name, *fs_type));
    f(&mut iter)
}

pub(crate) fn init() {
    FS_REGISTRY.call_once(|| {
        let singleton = FsRegistry::new();
        sysfs::systree_singleton()
            .root()
            .add_child(singleton.clone())
            .unwrap();
        singleton
    });
}

static FS_REGISTRY: Once<Arc<FsRegistry>> = Once::new();

pub(crate) struct FsCache<K: Eq + Ord + Clone + Send + Sync + 'static> {
    entries: Mutex<BTreeMap<K, WeakFsAndRoot>>,
}

#[derive(Clone)]
pub(crate) struct FsAndRoot {
    fs: Arc<dyn FileSystem>,
    root_dentry: Arc<Dentry>,
}

impl FsAndRoot {
    pub(crate) fn new(fs: Arc<dyn FileSystem>) -> Self {
        let root_dentry = Dentry::new_root(fs.root_inode());
        Self { fs, root_dentry }
    }

    pub(in crate::fs) fn into_parts(self) -> (Arc<dyn FileSystem>, Arc<Dentry>) {
        (self.fs, self.root_dentry)
    }

    pub(in crate::fs) fn fs(&self) -> &Arc<dyn FileSystem> {
        &self.fs
    }

    fn downgrade(&self) -> WeakFsAndRoot {
        WeakFsAndRoot {
            fs: Arc::downgrade(&self.fs),
            root_dentry: Arc::downgrade(&self.root_dentry),
        }
    }
}

struct WeakFsAndRoot {
    fs: Weak<dyn FileSystem>,
    root_dentry: Weak<Dentry>,
}

impl WeakFsAndRoot {
    fn upgrade(&mut self) -> Option<FsAndRoot> {
        let fs = self.fs.upgrade()?;
        let root_dentry = self.root_dentry.upgrade().unwrap_or_else(|| {
            let dentry = Dentry::new_root(fs.root_inode());
            self.root_dentry = Arc::downgrade(&dentry);
            dentry
        });
        Some(FsAndRoot { fs, root_dentry })
    }
}

impl<K: Eq + Ord + Clone + Send + Sync + 'static> FsCache<K> {
    pub(in crate::fs) const fn new() -> Self {
        Self {
            entries: Mutex::new(BTreeMap::new()),
        }
    }

    fn get_or_create<F>(&self, key: K, required_flags: FsFlags, create_fn: F) -> Result<FsAndRoot>
    where
        F: FnOnce() -> Result<Arc<dyn FileSystem>>,
    {
        let mut entries = self.entries.lock();
        match entries.entry(key) {
            Entry::Occupied(mut entry) => {
                if let Some(fs_and_root) = entry.get_mut().upgrade() {
                    let extant_readonly = fs_and_root.fs.flags().contains(FsFlags::RDONLY);
                    let requested_readonly = required_flags.contains(FsFlags::RDONLY);
                    if extant_readonly != requested_readonly {
                        return Err(errno!(
                            EBUSY,
                            "the read-only flag of the extant file system does not match"
                        ));
                    }
                    return Ok(fs_and_root);
                }

                let fs = match create_fn() {
                    Ok(fs) => fs,
                    Err(err) => {
                        entry.remove();
                        return Err(err);
                    }
                };
                let fs_and_root = FsAndRoot::new(fs);
                entry.insert(fs_and_root.downgrade());
                Ok(fs_and_root)
            }
            Entry::Vacant(entry) => {
                let fs = create_fn()?;
                let fs_and_root = FsAndRoot::new(fs);
                entry.insert(fs_and_root.downgrade());
                Ok(fs_and_root)
            }
        }
    }
}

struct FsRegistry {
    fs_table: Mutex<BTreeMap<&'static str, &'static dyn DynFsType>>,
    systree_fields: AttrLessBranchNodeFields<dyn SysObj, Self>,
}

impl Debug for FsRegistry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FsRegistry")
            .field("systree_fields", &self.systree_fields)
            .finish()
    }
}

impl FsRegistry {
    fn new() -> Arc<Self> {
        let name = SysStr::from("fs");
        Arc::new_cyclic(|weak_self| {
            let fs_table = Mutex::new(BTreeMap::new());
            let systree_fields = AttrLessBranchNodeFields::new(name, weak_self.clone());
            Self {
                fs_table,
                systree_fields,
            }
        })
    }

    fn register(&self, new_type: &'static dyn DynFsType) -> Result<()> {
        let mut fs_table = self.fs_table.lock();
        if fs_table.contains_key(new_type.name()) {
            return Err(errno!(EEXIST, "the file system type already exists"));
        }

        if let Some(node) = new_type.sysnode() {
            self.systree_fields.add_child(node)?;
        }

        fs_table.insert(new_type.name(), new_type);
        Ok(())
    }
}

inherit_sys_branch_node!(FsRegistry, systree_fields, {
    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }
});
