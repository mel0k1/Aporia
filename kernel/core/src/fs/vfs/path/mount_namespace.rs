use alloc::sync::UniqueArc;

use spin::Once;

use super::{
    UniqueMountId,
    mount::MountTreeCloneMode,
    mount_propagation::{MountTopology, PendingPropagationChanges},
    try_get_mnt_ns_inode,
};
use crate::{
    fs::{
        fs_impls::ramfs::RamFs,
        pseudofs::{NsCommonOps, NsType, NullFs, StashedDentry},
        vfs::{
            path::{Dentry, Mount, Path, PathResolver, PerMountFlags},
            registry::FsAndRoot,
        },
    },
    prelude::*,
    process::{UserNamespace, credentials::capabilities::CapSet, posix_thread::PosixThread},
    security::lsm::hooks as lsm_hooks,
};

pub(crate) struct MountNamespace {
    root: Option<Arc<Mount>>,
    owner: Arc<UserNamespace>,
    stashed_dentry: StashedDentry,
    mounts: SpinLock<BTreeMap<UniqueMountId, Weak<Mount>>>,
}

impl PartialEq for MountNamespace {
    fn eq(&self, other: &Self) -> bool {
        self.stashed_dentry == other.stashed_dentry
    }
}

impl Eq for MountNamespace {}

impl PartialOrd for MountNamespace {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MountNamespace {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.stashed_dentry.cmp(&other.stashed_dentry)
    }
}

impl MountNamespace {
    fn new_with_root<F>(owner: Arc<UserNamespace>, build_root_fn: F) -> Result<Arc<Self>>
    where
        F: FnOnce(&Weak<Self>) -> Result<Arc<Mount>>,
    {
        let mut new_ns = UniqueArc::new(Self {
            root: None,
            owner,
            stashed_dentry: StashedDentry::new(),
            mounts: SpinLock::new(BTreeMap::new()),
        });
        let root = build_root_fn(&UniqueArc::downgrade(&new_ns))?;
        new_ns.root = Some(root);
        let ns = UniqueArc::into_arc(new_ns);
        ns.register_existing_mount_tree();
        Ok(ns)
    }

    #[doc(hidden)]
    pub(crate) fn get_init_singleton() -> &'static Arc<MountNamespace> {
        static INIT: Once<Arc<MountNamespace>> = Once::new();

        INIT.call_once(|| {
            let owner = UserNamespace::get_init_singleton().clone();
            let nullfs = NullFs::singleton().clone();
            let rootfs = RamFs::new_rootfs();

            let namespace = Self::new_with_root(owner, |weak_ns| {
                Mount::new_root(nullfs, PerMountFlags::RDONLY, weak_ns.clone())
            })
            .expect("failed to create the nullfs root mount");

            let mut topology_guard = MountTopology::write_lock();
            let root_mount = namespace.root();
            root_mount
                .do_mount(
                    FsAndRoot::new(rootfs),
                    PerMountFlags::default(),
                    root_mount.root_dentry(),
                    Some("rootfs".to_string()),
                    &mut topology_guard,
                )
                .expect("failed to mount rootfs on nullfs");

            namespace
        })
    }

    pub(crate) fn root(&self) -> &Arc<Mount> {
        self.root.as_ref().unwrap()
    }

    pub(crate) fn new_path_resolver(&self) -> PathResolver {
        let root = Path::new_root(self.root().clone()).get_top_path();
        let cwd = Path::new_root(self.root().clone()).get_top_path();
        PathResolver::new(root, cwd)
    }

    pub(crate) fn new_clone(
        &self,
        owner: Arc<UserNamespace>,
        posix_thread: &PosixThread,
    ) -> Result<Arc<MountNamespace>> {
        lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
            owner.as_ref(),
            posix_thread,
            CapSet::SYS_ADMIN,
        ))?;

        let root_mount = self.root();
        Self::new_with_root(owner, |weak_ns| {
            let mut topology_guard = MountTopology::write_lock();
            let mut pending_changes = PendingPropagationChanges::default();
            let cloned_tree = root_mount.clone_mount_tree_deferred(
                root_mount.root_dentry(),
                weak_ns,
                true,
                MountTreeCloneMode::Namespace,
                &mut pending_changes,
                &topology_guard,
            )?;
            pending_changes.commit(&mut topology_guard);
            Ok(cloned_tree)
        })
    }

    pub(super) fn register_mount(&self, mount: &Arc<Mount>) {
        self.mounts
            .lock()
            .insert(mount.unique_id(), Arc::downgrade(mount));
    }

    pub(super) fn deregister_mount(&self, unique_id: UniqueMountId) {
        self.mounts.lock().remove(&unique_id);
    }

    pub(crate) fn lookup_by_unique_id(&self, unique_id: UniqueMountId) -> Option<Arc<Mount>> {
        let mount = {
            let mounts = self.mounts.lock();
            mounts.get(&unique_id).and_then(Weak::upgrade)?
        };

        let topology_guard = MountTopology::read_lock();
        mount
            .is_equal_or_descendant_of(self.root(), &topology_guard)
            .then_some(mount)
    }

    pub(crate) fn descendant_mounts_of(
        &self,
        parent: &Arc<Mount>,
    ) -> impl DoubleEndedIterator<Item = Arc<Mount>> {
        let snapshot: Vec<_> = {
            let guard = self.mounts.lock();
            guard.values().filter_map(|weak| weak.upgrade()).collect()
        };

        let topology_guard = MountTopology::read_lock();

        let descendants: Vec<_> = snapshot
            .into_iter()
            .filter(|mount| {
                !Arc::ptr_eq(mount, parent)
                    && mount.is_equal_or_descendant_of(parent, &topology_guard)
            })
            .collect();

        descendants.into_iter()
    }

    fn register_existing_mount_tree(self: &Arc<Self>) {
        let mut entries = Vec::new();
        let mut stack = vec![self.root().clone()];

        while let Some(mount) = stack.pop() {
            entries.push((mount.unique_id(), Arc::downgrade(&mount)));

            let children: Vec<_> = mount.children.read().values().cloned().collect();
            stack.extend(children);
        }

        let mut mounts = self.mounts.lock();
        for (id, weak_mount) in entries {
            mounts.insert(id, weak_mount);
        }
    }

    pub(crate) fn sync(&self) -> Result<()> {
        let mut visited_filesystems = hashbrown::HashSet::new();

        self.root().try_walk_tree(|current_mount| {
            let fs_ptr = Arc::as_ptr(current_mount.fs());
            if visited_filesystems.insert(fs_ptr) {
                current_mount.sync()?;
            }

            Ok(())
        })?;

        Ok(())
    }

    pub(crate) fn owns(self: &Arc<Self>, mount: &Mount) -> bool {
        mount.mnt_ns().as_ptr() == Arc::as_ptr(self)
    }

    pub(super) fn would_form_mnt_ns_loop(&self, dentry: &Dentry) -> bool {
        let Some(mnt_ns_inode) = try_get_mnt_ns_inode(dentry) else {
            return false;
        };

        self >= mnt_ns_inode.ns().as_ref()
    }

    pub(super) fn check_no_mnt_ns_loop_in_tree(
        &self,
        root_mount: &Arc<Mount>,
        _topology: &MountTopology,
    ) -> Result<()> {
        let mut checked_root_dentries = BTreeSet::new();

        root_mount.try_walk_tree(|mount| {
            let root_dentry = mount.root_dentry();
            if checked_root_dentries.insert(root_dentry.key())
                && self.would_form_mnt_ns_loop(root_dentry)
            {
                return Err(errno!(
                    ELOOP,
                    "the mount tree contains a mount namespace file that would create a namespace loop"
                ));
            }
            Ok(())
        })?;

        Ok(())
    }
}

impl Drop for MountNamespace {
    fn drop(&mut self) {
        let Some(root) = self.root.as_ref() else {
            return;
        };

        let mut topology_guard = MountTopology::write_lock();

        let mut worklist = VecDeque::new();
        worklist.push_back(root.clone());
        while let Some(current_mount) = worklist.pop_front() {
            current_mount.clear_propagation(&mut topology_guard);
            let children = core::mem::take(&mut *current_mount.children.write());
            for (_, child) in children {
                child.clear_topology_link(&mut topology_guard);
                worklist.push_back(child);
            }
        }
    }
}

impl NsCommonOps for MountNamespace {
    const TYPE: NsType = NsType::Mnt;

    fn owner_user_ns(&self) -> Option<&Arc<UserNamespace>> {
        Some(&self.owner)
    }

    fn parent(&self) -> Result<&Arc<Self>> {
        Err(errno!(
            EINVAL,
            "a mount namespace does not have a parent namespace"
        ))
    }

    fn stashed_dentry(&self) -> &StashedDentry {
        &self.stashed_dentry
    }
}
