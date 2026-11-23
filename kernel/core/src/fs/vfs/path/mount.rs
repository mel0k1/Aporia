use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;
use hashbrown::{HashMap, HashSet};
use sparse_id_alloc::SparseIdAlloc;

use super::{
    mount_propagation::{
        MountPropType, MountPropagation, MountTopology, PeerGroup, PeerGroupId,
        PendingPropagationChanges,
    },
    try_get_mnt_ns_inode,
};
use crate::{
    fs::{
        file::InodeType,
        vfs::{
            file_system::{FileSystem, FsFlags},
            path::{
                Path,
                dentry::{Dentry, DentryKey},
                mount_namespace::MountNamespace,
            },
            registry::FsAndRoot,
        },
    },
    prelude::*,
};

pub(crate) struct Mount {
    id: MountId,
    root_dentry: Arc<Dentry>,
    mountpoint: RwLock<Option<Arc<Dentry>>>,
    fs: Arc<dyn FileSystem>,
    source: Option<String>,
    parent: RwLock<Option<Weak<Mount>>>,
    pub(super) children: RwLock<HashMap<DentryKey, Arc<Self>>>,
    mnt_ns: Weak<MountNamespace>,
    propagation: RwLock<MountPropagation>,
    flags: AtomicPerMountFlags,
    this: Weak<Self>,
}

impl Mount {
    pub(super) fn try_walk_tree(
        self: &Arc<Self>,
        mut visit_fn: impl FnMut(Arc<Self>) -> Result<()>,
    ) -> Result<()> {
        let mut pending = vec![self.clone()];

        while let Some(mount) = pending.pop() {
            let children = mount.children.read();
            pending.extend(children.values().cloned());
            drop(children);

            visit_fn(mount)?;
        }

        Ok(())
    }

    pub(in crate::fs) fn new_root(
        fs: Arc<dyn FileSystem>,
        flags: PerMountFlags,
        mnt_ns: Weak<MountNamespace>,
    ) -> Result<Arc<Self>> {
        let source = fs.name().to_string();
        let root_dentry = Dentry::new_root(fs.root_inode());
        Self::new(root_dentry, fs, flags, None, mnt_ns, Some(source))
    }

    pub(in crate::fs) fn new_pseudo(fs: Arc<dyn FileSystem>) -> Result<Arc<Self>> {
        let root_dentry = Dentry::new_root(fs.root_inode());
        Self::new(
            root_dentry,
            fs,
            PerMountFlags::KERNMOUNT,
            None,
            Weak::new(),
            None,
        )
    }

    pub(crate) fn new_detached(
        fs_and_root: FsAndRoot,
        flags: PerMountFlags,
        mnt_ns: Weak<MountNamespace>,
        source: Option<String>,
    ) -> Result<Arc<Self>> {
        let (fs, root_dentry) = fs_and_root.into_parts();
        Self::new(root_dentry, fs, flags, None, mnt_ns, source)
    }

    fn new(
        root_dentry: Arc<Dentry>,
        fs: Arc<dyn FileSystem>,
        flags: PerMountFlags,
        parent_mount: Option<Weak<Mount>>,
        mnt_ns: Weak<MountNamespace>,
        source: Option<String>,
    ) -> Result<Arc<Self>> {
        let id = MountId::alloc().ok_or_else(|| errno!(ENOMEM, "mount ID space exhausted"))?;

        let mount = Arc::new_cyclic(|weak_self| Self {
            id,
            root_dentry,
            mountpoint: RwLock::new(None),
            fs,
            source,
            parent: RwLock::new(parent_mount),
            children: RwLock::new(HashMap::new()),
            mnt_ns,
            propagation: RwLock::new(MountPropagation::default()),
            flags: AtomicPerMountFlags::new(flags),
            this: weak_self.clone(),
        });
        if let Some(ns) = mount.mnt_ns.upgrade() {
            ns.register_mount(&mount);
        }
        Ok(mount)
    }

    pub(crate) fn id(&self) -> RecyclableMountId {
        self.id.recyclable_id()
    }

    pub(crate) fn unique_id(&self) -> UniqueMountId {
        self.id.unique_id()
    }

    pub(in crate::fs) fn source(&self) -> Option<&str> {
        self.fs.source().or(self.source.as_deref())
    }

    pub(super) fn do_mount(
        self: &Arc<Self>,
        fs_and_root: FsAndRoot,
        flags: PerMountFlags,
        mountpoint: &Arc<Dentry>,
        source: Option<String>,
        topology: &mut MountTopology,
    ) -> Result<Arc<Self>> {
        if mountpoint.type_() != InodeType::Dir {
            return Err(errno!(ENOTDIR));
        }

        let child_mount = Self::new_detached(fs_and_root, flags, self.mnt_ns.clone(), source)?;
        let target_path = Path::new(self.clone(), mountpoint.clone());
        let pending_changes = PendingPropagationChanges::default();
        let attachment =
            child_mount.prepare_mount_tree_attachment(target_path, pending_changes, topology)?;
        attachment.attach(topology);

        Ok(child_mount)
    }

    pub(super) fn do_unmount(
        self: &Arc<Self>,
        mountpoint: &Arc<Dentry>,
        topology: &mut MountTopology,
    ) -> Result<Arc<Self>> {
        let child_mount = self
            .get(mountpoint)
            .ok_or_else(|| errno!(ENOENT, "can not find child mount"))?;
        let parent = child_mount
            .parent()
            .and_then(|parent| parent.upgrade())
            .ok_or_else(|| errno!(EINVAL, "the root mount cannot be unmounted"))?;

        let mut unmounted_mounts = vec![child_mount];
        for receiver in topology.propagation_receivers(&parent, mountpoint) {
            if let Some(child) = receiver.get(mountpoint) {
                unmounted_mounts.push(child);
            }
        }

        for target in &unmounted_mounts {
            target
                .try_walk_tree(|mount| {
                    mount.clear_propagation(topology);
                    Ok(())
                })
                .unwrap();
            target.detach_from_parent(topology);
        }

        Ok(unmounted_mounts.into_iter().next().unwrap())
    }

    fn clone_mount(
        &self,
        root_dentry: &Arc<Dentry>,
        propagation: MountPropagation,
        new_ns: &Weak<MountNamespace>,
    ) -> Result<Arc<Self>> {
        let id = MountId::alloc().ok_or_else(|| errno!(ENOMEM, "mount ID space exhausted"))?;

        let mount = Arc::new_cyclic(|weak_self| Self {
            id,
            root_dentry: root_dentry.clone(),
            mountpoint: RwLock::new(None),
            fs: self.fs.clone(),
            source: self.source.clone(),
            parent: RwLock::new(None),
            children: RwLock::new(HashMap::new()),
            mnt_ns: new_ns.clone(),
            propagation: RwLock::new(propagation),
            flags: AtomicPerMountFlags::new(self.flags.load(Ordering::Relaxed)),
            this: weak_self.clone(),
        });
        if let Some(ns) = mount.mnt_ns.upgrade() {
            ns.register_mount(&mount);
        }
        Ok(mount)
    }

    pub(super) fn clone_mount_tree_deferred(
        self: &Arc<Self>,
        root_dentry: &Arc<Dentry>,
        new_ns: &Weak<MountNamespace>,
        recursive: bool,
        clone_mode: MountTreeCloneMode,
        pending_changes: &mut PendingPropagationChanges,
        _topology: &MountTopology,
    ) -> Result<Arc<Self>> {
        let skip_unbindable = clone_mode == MountTreeCloneMode::Bind;
        if skip_unbindable && self.is_unbindable() {
            return Err(errno!(EINVAL, "the source mount is unbindable"));
        }
        if clone_mode != MountTreeCloneMode::Bind && try_get_mnt_ns_inode(root_dentry).is_some() {
            return Err(errno!(
                EINVAL,
                "a mount namespace file cannot be the root of this mount tree clone"
            ));
        }

        let mut clone_mount_fn = |source_mount: &Mount,
                                  cloned_root_dentry: &Arc<Dentry>|
         -> Result<Arc<Mount>> {
            let cloned_propagation = match clone_mode {
                MountTreeCloneMode::SharedPropagation => {
                    pending_changes.make_shared_for_clone(source_mount)?
                }
                MountTreeCloneMode::SlavePropagation => {
                    let source_propagation = pending_changes.make_shared_for_clone(source_mount)?;
                    let source_peer_group_id = source_propagation
                        .peer_group_id()
                        .expect("the propagation source must be shared");
                    MountPropagation::slave(source_peer_group_id)
                }
                MountTreeCloneMode::Bind | MountTreeCloneMode::Namespace => {
                    source_mount.propagation()
                }
            };

            let cloned_mount =
                source_mount.clone_mount(cloned_root_dentry, cloned_propagation, new_ns)?;
            pending_changes.add_register_op(cloned_mount.clone());
            Ok(cloned_mount)
        };

        let new_root_mount = clone_mount_fn(self, root_dentry)?;
        let mut cloned_mounts = vec![(self.this(), new_root_mount.clone())];
        if recursive {
            let mut work_idx = 0;
            while work_idx < cloned_mounts.len() {
                let (old_parent_mount, new_parent_mount) = cloned_mounts[work_idx].clone();
                let old_children = old_parent_mount
                    .children
                    .read()
                    .values()
                    .cloned()
                    .collect::<Vec<_>>();
                for old_child_mount in old_children {
                    if skip_unbindable && old_child_mount.is_unbindable() {
                        continue;
                    }
                    if clone_mode != MountTreeCloneMode::Bind
                        && try_get_mnt_ns_inode(old_child_mount.root_dentry()).is_some()
                    {
                        continue;
                    }

                    let mountpoint = old_child_mount.mountpoint().unwrap();
                    if !mountpoint.is_equal_or_descendant_of(new_parent_mount.root_dentry()) {
                        continue;
                    }

                    let new_child_mount =
                        clone_mount_fn(&old_child_mount, old_child_mount.root_dentry())?;

                    let key = mountpoint.key();
                    let replaced = new_parent_mount
                        .children
                        .write()
                        .insert(key, new_child_mount.clone());
                    new_child_mount.set_parent(Some(&new_parent_mount));
                    new_child_mount.set_mountpoint(mountpoint);
                    cloned_mounts.push((old_child_mount, new_child_mount));

                    drop(replaced);
                }
                work_idx += 1;
            }
        }

        Ok(new_root_mount)
    }

    pub(super) fn set_propagation_deferred(
        self: &Arc<Self>,
        prop: MountPropType,
        recursive: bool,
        pending_changes: &mut PendingPropagationChanges,
        _topology: &MountTopology,
    ) -> Result<()> {
        let mut queue_transition_fn = |mount: Arc<Mount>| -> Result<()> {
            let new_propagation = match prop {
                MountPropType::Shared => {
                    let old_propagation = mount.propagation();
                    if old_propagation.is_shared() {
                        return Ok(());
                    } else {
                        let peer_group = PeerGroup::new()?;
                        let new_propagation = old_propagation.make_shared(peer_group.id());
                        pending_changes.add_peer_group(mount.unique_id(), peer_group);
                        new_propagation
                    }
                }
                MountPropType::Slave => {
                    pending_changes.add_make_slave_op(mount);
                    return Ok(());
                }
                MountPropType::Private => MountPropagation::private(),
                MountPropType::Unbindable => MountPropagation::unbindable(),
            };
            pending_changes.add_set_op(mount, new_propagation);
            Ok(())
        };
        if recursive {
            self.try_walk_tree(queue_transition_fn)?;
        } else {
            queue_transition_fn(self.this())?;
        }

        Ok(())
    }

    pub(in crate::fs) fn propagation_mountinfo_fields(&self) -> String {
        let propagation = self.propagation.read();
        let peer_group_id = propagation.peer_group_id();
        let master_id = propagation.master_id();
        match (peer_group_id, master_id) {
            (Some(peer_group_id), Some(master_id)) => {
                alloc::format!(" shared:{} master:{}", peer_group_id, master_id)
            }
            (Some(peer_group_id), None) => alloc::format!(" shared:{}", peer_group_id),
            (None, Some(master_id)) => alloc::format!(" master:{}", master_id),
            (None, None) => String::new(),
        }
    }

    pub(super) fn is_unbindable(&self) -> bool {
        self.propagation.read().is_unbindable()
    }

    pub(super) fn is_shared(&self) -> bool {
        self.propagation.read().is_shared()
    }

    pub(super) fn propagation(&self) -> MountPropagation {
        *self.propagation.read()
    }

    pub(super) fn set_propagation_master(&self, master_id: Option<PeerGroupId>) {
        self.propagation.write().set_master_id(master_id);
    }

    pub(super) fn publish_propagation(
        self: &Arc<Self>,
        propagation: MountPropagation,
        topology: &mut MountTopology,
    ) {
        let old_propagation = self.propagation();
        let propagation = topology.replace_mount_propagation(self, old_propagation, propagation);
        *self.propagation.write() = propagation;
    }

    pub(super) fn clear_propagation(&self, topology: &mut MountTopology) {
        let old_propagation = self.propagation();
        topology.unregister_mount_propagation(self, old_propagation);
        *self.propagation.write() = MountPropagation::default();
    }

    pub(super) fn detach_from_parent(&self, topology: &mut MountTopology) {
        if let Some(parent) = self.parent() {
            let parent = parent.upgrade().unwrap();
            let child = parent
                .children
                .write()
                .remove(&self.mountpoint().unwrap().key());

            if let Some(child) = child {
                child.clear_topology_link(topology);
            }
        }
    }

    pub(super) fn clear_topology_link(&self, _topology: &mut MountTopology) {
        self.set_parent(None);
        self.clear_mountpoint();
    }

    fn attach_to_path(&self, target_path: Path, _topology: &mut MountTopology) {
        let key = target_path.dentry.key();
        let replaced = target_path
            .mount_node()
            .children
            .write()
            .insert(key, self.this());
        self.set_parent(Some(target_path.mount_node()));
        self.set_mountpoint(target_path.dentry);

        drop(replaced);
    }

    pub(super) fn graft_mount_tree(&self, target_path: Path, topology: &mut MountTopology) {
        self.detach_from_parent(topology);
        self.attach_to_path(target_path, topology);
    }

    pub(super) fn prepare_mount_tree_attachment(
        self: &Arc<Self>,
        target_path: Path,
        mut pending_changes: PendingPropagationChanges,
        topology: &MountTopology,
    ) -> Result<PendingMountTreeAttachment> {
        let mut copies = Vec::new();
        let mut propagation_layers = Vec::new();
        if let Some(peer_group_id) = target_path.mount_node().propagation().peer_group_id() {
            propagation_layers.push((
                target_path.mount_node().clone(),
                self.clone(),
                peer_group_id,
            ));
        }

        while let Some((source_parent, source_mount, peer_group_id)) = propagation_layers.pop() {
            let copies_before_layer = copies.len();

            for receiver in topology.group_peers_of(peer_group_id) {
                if receiver.unique_id() == source_parent.unique_id()
                    || !target_path
                        .dentry()
                        .is_equal_or_descendant_of(receiver.root_dentry())
                {
                    continue;
                }
                let copy = source_mount.clone_mount_tree_deferred(
                    source_mount.root_dentry(),
                    receiver.mnt_ns(),
                    true,
                    MountTreeCloneMode::SharedPropagation,
                    &mut pending_changes,
                    topology,
                )?;
                copies.push((receiver, copy));
            }

            let mut seen_slave_peer_groups = HashSet::new();
            for receiver in topology.slave_mounts_of(peer_group_id) {
                if !target_path
                    .dentry()
                    .is_equal_or_descendant_of(receiver.root_dentry())
                {
                    continue;
                }
                let receiver_peer_group_id = receiver.propagation().peer_group_id();
                if receiver_peer_group_id
                    .is_some_and(|peer_group_id| !seen_slave_peer_groups.insert(peer_group_id))
                {
                    continue;
                }

                let copy = source_mount.clone_mount_tree_deferred(
                    source_mount.root_dentry(),
                    receiver.mnt_ns(),
                    true,
                    MountTreeCloneMode::SlavePropagation,
                    &mut pending_changes,
                    topology,
                )?;
                if let Some(peer_group_id) = receiver_peer_group_id {
                    propagation_layers.push((receiver.clone(), copy.clone(), peer_group_id));
                }
                copies.push((receiver, copy));
            }

            if copies.len() == copies_before_layer {
                source_mount.set_propagation_deferred(
                    MountPropType::Shared,
                    true,
                    &mut pending_changes,
                    topology,
                )?;
            }
        }

        Ok(PendingMountTreeAttachment {
            mount: self.clone(),
            target_path,
            copies,
            pending_changes,
        })
    }

    pub(super) fn get(&self, mountpoint: &Dentry) -> Option<Arc<Self>> {
        self.children.read().get(&mountpoint.key()).cloned()
    }

    pub(in crate::fs) fn root_dentry(&self) -> &Arc<Dentry> {
        &self.root_dentry
    }

    pub(in crate::fs) fn mountpoint(&self) -> Option<Arc<Dentry>> {
        self.mountpoint.read().clone()
    }

    fn set_mountpoint(&self, dentry: Arc<Dentry>) {
        let _old = {
            let mut mountpoint = self.mountpoint.write();
            if let Some(old) = mountpoint.as_deref() {
                old.dec_mount_count();
            }

            dentry.inc_mount_count();
            mountpoint.replace(dentry)
        };
    }

    fn clear_mountpoint(&self) {
        let _old = {
            let mut mountpoint = self.mountpoint.write();
            if let Some(old) = mountpoint.as_deref() {
                old.dec_mount_count();
            }

            mountpoint.take()
        };
    }

    pub(super) fn sync(&self) -> Result<()> {
        self.fs.sync()?;
        Ok(())
    }

    pub(super) fn remount(
        &self,
        mount_flags: PerMountFlags,
        fs_flags: Option<FsFlags>,
        data: Option<&str>,
        ctx: &Context,
        _topology: &mut MountTopology,
    ) -> Result<()> {
        if let Some(flags) = fs_flags {
            self.fs.set_fs_flags(flags, data, ctx)?;
        }

        const ATIME_MASK: PerMountFlags = PerMountFlags::NOATIME
            .union(PerMountFlags::RELATIME)
            .union(PerMountFlags::STRICTATIME);

        let need_inherit_atime = !mount_flags.intersects(ATIME_MASK | PerMountFlags::NODIRATIME);

        if need_inherit_atime {
            let old_flags = self.flags.load(Ordering::Relaxed);
            let new_flags = mount_flags | (old_flags & ATIME_MASK);
            self.flags.store(new_flags, Ordering::Relaxed);
        } else {
            self.flags.store(mount_flags, Ordering::Relaxed);
        }

        Ok(())
    }

    pub(in crate::fs) fn parent(&self) -> Option<Weak<Self>> {
        self.parent.read().as_ref().cloned()
    }

    pub(super) fn is_equal_or_descendant_of(
        &self,
        ancestor: &Arc<Self>,
        _topology: &MountTopology,
    ) -> bool {
        let mut current = self.this();
        loop {
            if Arc::ptr_eq(&current, ancestor) {
                return true;
            }

            let Some(parent) = current.parent().and_then(|parent| parent.upgrade()) else {
                return false;
            };
            current = parent;
        }
    }

    pub(super) fn mnt_ns(&self) -> &Weak<MountNamespace> {
        &self.mnt_ns
    }

    pub(crate) fn fs(&self) -> &Arc<dyn FileSystem> {
        &self.fs
    }

    pub(crate) fn flags(&self) -> PerMountFlags {
        self.flags.load(Ordering::Relaxed)
    }

    fn set_parent(&self, mount: Option<&Arc<Mount>>) {
        let mut parent = self.parent.write();
        *parent = mount.map(Arc::downgrade);
    }

    pub(super) fn find_corresponding_mount(
        &self,
        mnt_ns: &Arc<MountNamespace>,
        _topology: &MountTopology,
    ) -> Option<Arc<Self>> {
        let mut ancestors = VecDeque::new();
        let mut current = self.this();
        while let Some(weak_parent) = current.parent() {
            ancestors.push_front(current.clone());
            current = weak_parent.upgrade().unwrap();
        }

        let mut target_mount = mnt_ns.root().clone();
        while let Some(ancestor) = ancestors.pop_front() {
            let mount_point = ancestor.mountpoint().unwrap();
            let child_mount = target_mount
                .children
                .read()
                .get(&mount_point.key())
                .cloned()?;
            target_mount = child_mount;
        }

        Some(target_mount)
    }

    pub(super) fn this(&self) -> Arc<Self> {
        self.this.upgrade().unwrap()
    }
}

impl Debug for Mount {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("Mount")
            .field("root", &self.root_dentry)
            .field("mountpoint", &self.mountpoint)
            .field("fs", &self.fs)
            .finish()
    }
}

impl Drop for Mount {
    fn drop(&mut self) {
        self.clear_mountpoint();
        if let Some(ns) = self.mnt_ns.upgrade() {
            ns.deregister_mount(self.id.unique_id());
        }
    }
}

pub(super) struct PendingMountTreeAttachment {
    mount: Arc<Mount>,
    target_path: Path,
    copies: Vec<(Arc<Mount>, Arc<Mount>)>,
    pending_changes: PendingPropagationChanges,
}

impl PendingMountTreeAttachment {
    pub(super) fn attach(self, topology: &mut MountTopology) {
        self.mount.detach_from_parent(topology);

        for (receiver, copy) in self.copies {
            let mount_point = self.target_path.dentry.clone();
            debug_assert!(Arc::ptr_eq(&receiver.fs, &mount_point.inode().fs()));
            if let Some(existing_mount) = receiver.get(&mount_point) {
                let copy_root = Path::new(copy.clone(), copy.root_dentry().clone()).get_top_path();
                existing_mount.graft_mount_tree(copy_root, topology);
            }
            let target_path = Path::new(receiver, mount_point);

            copy.attach_to_path(target_path, topology);
        }
        self.mount.attach_to_path(self.target_path, topology);
        self.pending_changes.commit(topology);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MountTreeCloneMode {
    Bind,
    Namespace,
    SharedPropagation,
    SlavePropagation,
}

pub(in crate::fs) type RecyclableMountId = u32;

pub(in crate::fs) type UniqueMountId = u64;

static MOUNT_ID_ALLOCATOR: SpinLock<SparseIdAlloc> =
    SpinLock::new(SparseIdAlloc::new(1, i32::MAX as u32));

pub(crate) const MNT_UNIQUE_ID_MIN: u64 = (1 << 31) + 1;

static NEXT_FREE_UNIQUE_ID: AtomicU64 = AtomicU64::new(MNT_UNIQUE_ID_MIN);

pub(super) fn init() {}

pub(super) struct MountId {
    recyclable_id: RecyclableMountId,
    unique_id: UniqueMountId,
}

impl MountId {
    pub(super) fn alloc() -> Option<Self> {
        let recyclable_id = MOUNT_ID_ALLOCATOR.lock().alloc()?;
        let unique_id = NEXT_FREE_UNIQUE_ID.fetch_add(1, Ordering::Relaxed);
        Some(Self {
            recyclable_id,
            unique_id,
        })
    }

    pub(super) fn recyclable_id(&self) -> RecyclableMountId {
        self.recyclable_id
    }

    pub(super) fn unique_id(&self) -> UniqueMountId {
        self.unique_id
    }
}

impl Drop for MountId {
    fn drop(&mut self) {
        MOUNT_ID_ALLOCATOR.lock().free(self.recyclable_id);
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub(crate) struct PerMountFlags: u32 {
        const RDONLY      = 1 << 0;
        const NOSUID      = 1 << 1;
        const NODEV       = 1 << 2;
        const NOEXEC      = 1 << 3;
        const NOSYMFOLLOW = 1 << 8;
        const NOATIME     = 1 << 10;
        const NODIRATIME  = 1 << 11;
        const RELATIME    = 1 << 21;
        const KERNMOUNT   = 1 << 22;
        const STRICTATIME = 1 << 24;
    }
}

impl Default for PerMountFlags {
    fn default() -> Self {
        let empty = Self::empty();
        empty | Self::RELATIME
    }
}

impl PerMountFlags {
    fn atime_policy(&self) -> AtimePolicy {
        if self.contains(PerMountFlags::STRICTATIME) {
            AtimePolicy::Strictatime
        } else if self.contains(PerMountFlags::NOATIME) {
            AtimePolicy::Noatime
        } else {
            AtimePolicy::Relatime
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AtimePolicy {
    Relatime,
    Noatime,
    Strictatime,
}

impl core::fmt::Display for PerMountFlags {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.contains(PerMountFlags::RDONLY) {
            write!(f, "ro")?;
        } else {
            write!(f, "rw")?;
        };
        if self.contains(PerMountFlags::NOSUID) {
            write!(f, ",nosuid")?;
        }
        if self.contains(PerMountFlags::NODEV) {
            write!(f, ",nodev")?;
        }
        if self.contains(PerMountFlags::NOEXEC) {
            write!(f, ",noexec")?;
        }
        if self.contains(PerMountFlags::NODIRATIME) {
            write!(f, ",nodiratime")?;
        }
        let atime_policy = match self.atime_policy() {
            AtimePolicy::Relatime => "relatime",
            AtimePolicy::Noatime => "noatime",
            AtimePolicy::Strictatime => "strictatime",
        };
        write!(f, ",{}", atime_policy)
    }
}

impl From<u32> for PerMountFlags {
    fn from(value: u32) -> Self {
        Self::from_bits_truncate(value)
    }
}

impl From<PerMountFlags> for u32 {
    fn from(value: PerMountFlags) -> Self {
        value.bits()
    }
}

define_atomic_version_of_integer_like_type!(PerMountFlags, {
    #[derive(Debug)]
    pub(crate) struct AtomicPerMountFlags(AtomicU32);
});
