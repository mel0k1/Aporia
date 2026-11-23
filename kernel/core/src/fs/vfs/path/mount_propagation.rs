use hashbrown::HashSet;
use ostd::sync::{RwMutexReadGuard, RwMutexWriteGuard};
use sparse_id_alloc::SparseIdAlloc;

use super::{UniqueMountId, dentry::Dentry, mount::Mount};
use crate::prelude::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct MountPropagation {
    peer_group_id: Option<PeerGroupId>,
    master_id: Option<PeerGroupId>,
    bindability: MountBindability,
}

pub(super) type PeerGroupId = u32;

impl Default for MountPropagation {
    fn default() -> Self {
        Self::private()
    }
}

impl MountPropagation {
    pub(super) fn private() -> Self {
        Self {
            peer_group_id: None,
            master_id: None,
            bindability: MountBindability::Bindable,
        }
    }

    pub(super) fn slave(master_id: PeerGroupId) -> Self {
        Self {
            peer_group_id: None,
            master_id: Some(master_id),
            bindability: MountBindability::Bindable,
        }
    }

    pub(super) fn shared(peer_group_id: PeerGroupId, master_id: Option<PeerGroupId>) -> Self {
        debug_assert_ne!(master_id, Some(peer_group_id));
        Self {
            peer_group_id: Some(peer_group_id),
            master_id,
            bindability: MountBindability::Bindable,
        }
    }

    pub(super) fn unbindable() -> Self {
        Self {
            peer_group_id: None,
            master_id: None,
            bindability: MountBindability::Unbindable,
        }
    }

    pub(super) fn peer_group_id(&self) -> Option<PeerGroupId> {
        self.peer_group_id
    }

    pub(super) fn master_id(&self) -> Option<PeerGroupId> {
        self.master_id
    }

    pub(super) fn is_shared(&self) -> bool {
        self.peer_group_id.is_some()
    }

    pub(super) fn is_unbindable(&self) -> bool {
        self.bindability == MountBindability::Unbindable
    }

    pub(super) fn set_master_id(&mut self, master_id: Option<PeerGroupId>) {
        self.master_id = master_id;
    }

    pub(super) fn make_shared(self, peer_group_id: PeerGroupId) -> Self {
        if self.is_shared() {
            return self;
        }

        Self::shared(peer_group_id, self.master_id())
    }

    pub(super) fn make_slave(self) -> Self {
        let Some(peer_group_id) = self.peer_group_id() else {
            return self;
        };

        Self::slave(peer_group_id)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum MountBindability {
    #[default]
    Bindable,
    Unbindable,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum MountPropType {
    #[default]
    Private,
    Slave,
    Shared,
    Unbindable,
}

pub(in crate::fs) struct MountTopology {
    peer_groups: BTreeMap<PeerGroupId, PeerGroup>,
}

static MOUNT_TOPOLOGY: RwMutex<MountTopology> = RwMutex::new(MountTopology {
    peer_groups: BTreeMap::new(),
});

impl MountTopology {
    pub(super) fn write_lock() -> RwMutexWriteGuard<'static, Self> {
        MOUNT_TOPOLOGY.write()
    }

    pub(super) fn read_lock() -> RwMutexReadGuard<'static, Self> {
        MOUNT_TOPOLOGY.read()
    }

    pub(super) fn propagation_receivers(
        &self,
        source_parent: &Arc<Mount>,
        mountpoint: &Arc<Dentry>,
    ) -> Vec<Arc<Mount>> {
        let mut visited = HashSet::new();
        visited.insert(source_parent.unique_id());
        let mut try_visit_receiver = |receiver: &Arc<Mount>| {
            visited.insert(receiver.unique_id())
                && mountpoint.is_equal_or_descendant_of(receiver.root_dentry())
        };

        let mut receivers = self
            .direct_propagation_receivers(source_parent)
            .filter(&mut try_visit_receiver)
            .collect::<Vec<_>>();

        let mut work_idx = 0;
        while work_idx < receivers.len() {
            let direct_receivers = self.direct_propagation_receivers(&receivers[work_idx]);
            receivers.extend(direct_receivers.filter(&mut try_visit_receiver));
            work_idx += 1;
        }

        receivers
    }

    pub(super) fn group_peers_of(
        &self,
        peer_group_id: PeerGroupId,
    ) -> impl Iterator<Item = Arc<Mount>> + '_ {
        self.peer_groups
            .get(&peer_group_id)
            .into_iter()
            .flat_map(|group| group.peers.values())
            .filter_map(Weak::upgrade)
    }

    pub(super) fn slave_mounts_of(
        &self,
        peer_group_id: PeerGroupId,
    ) -> impl Iterator<Item = Arc<Mount>> + '_ {
        self.peer_groups
            .get(&peer_group_id)
            .into_iter()
            .flat_map(|group| group.slaves.values())
            .filter_map(Weak::upgrade)
    }

    fn remove_peer(
        &mut self,
        peer_id: UniqueMountId,
        propagation: &MountPropagation,
    ) -> Option<PeerGroupId> {
        let peer_group_id = propagation
            .peer_group_id()
            .expect("peer propagation must have a peer group");
        let upstream_master_id = propagation.master_id();
        let peer_group = self.peer_groups.get_mut(&peer_group_id)?;
        let removed_peer = peer_group.peers.remove(&peer_id);
        debug_assert!(removed_peer.is_some());

        if !peer_group.peers.is_empty() {
            return None;
        }

        let mut peer_group = self
            .peer_groups
            .remove(&peer_group_id)
            .expect("peer group must still exist");
        debug_assert!(peer_group.peers.is_empty());

        for (mount_id, slave) in core::mem::take(&mut peer_group.slaves) {
            let Some(slave_mount) = slave.upgrade() else {
                continue;
            };

            slave_mount.set_propagation_master(upstream_master_id);
            if let Some(upstream_master_id) = upstream_master_id {
                let upstream_group = self
                    .peer_groups
                    .get_mut(&upstream_master_id)
                    .expect("upstream master peer group must exist");
                let old_slave = upstream_group.slaves.insert(mount_id, slave);
                debug_assert!(old_slave.is_none());
            }
        }

        upstream_master_id
    }

    fn insert_peer_group(&mut self, peer_group: PeerGroup) {
        let peer_group_id = peer_group.id;
        let old_group = self.peer_groups.insert(peer_group_id, peer_group);
        debug_assert!(old_group.is_none());
    }

    fn register_mount_propagation(&mut self, mount: &Arc<Mount>) {
        let propagation = mount.propagation();

        if let Some(peer_group_id) = propagation.peer_group_id() {
            let peer_group = self
                .peer_groups
                .get_mut(&peer_group_id)
                .expect("peer group must exist");
            let old_peer = peer_group
                .peers
                .insert(mount.unique_id(), Arc::downgrade(mount));
            debug_assert!(old_peer.is_none());
        }

        if let Some(master_id) = propagation.master_id() {
            let master_group = self
                .peer_groups
                .get_mut(&master_id)
                .expect("master peer group must exist");
            let old_slave = master_group
                .slaves
                .insert(mount.unique_id(), Arc::downgrade(mount));
            debug_assert!(old_slave.is_none());
        }
    }

    pub(super) fn replace_mount_propagation(
        &mut self,
        mount: &Arc<Mount>,
        old: MountPropagation,
        mut new: MountPropagation,
    ) -> MountPropagation {
        let mut old_master_removed = false;
        if old.master_id() != new.master_id()
            && let Some(old_master_id) = old.master_id()
        {
            let old_master_group = self
                .peer_groups
                .get_mut(&old_master_id)
                .expect("master peer group must exist");
            let old_slave = old_master_group.slaves.remove(&mount.unique_id());
            debug_assert!(old_slave.is_some());
            old_master_removed = true;
        }

        if old.peer_group_id() != new.peer_group_id()
            && let Some(old_peer_group_id) = old.peer_group_id()
        {
            let old_group_existed = self.peer_groups.contains_key(&old_peer_group_id);
            let reparented_master_id = self.remove_peer(mount.unique_id(), &old);
            if old_group_existed
                && !self.peer_groups.contains_key(&old_peer_group_id)
                && new.master_id() == Some(old_peer_group_id)
            {
                new.set_master_id(reparented_master_id);
            }
        }

        if let Some(new_master_id) = new.master_id()
            && (old_master_removed || old.master_id() != Some(new_master_id))
        {
            let new_master_group = self
                .peer_groups
                .get_mut(&new_master_id)
                .expect("master peer group must exist");
            let old_slave = new_master_group
                .slaves
                .insert(mount.unique_id(), Arc::downgrade(mount));
            debug_assert!(old_slave.is_none());
        }

        if let Some(new_peer_group_id) = new.peer_group_id()
            && old.peer_group_id() != Some(new_peer_group_id)
        {
            let new_peer_group = self
                .peer_groups
                .get_mut(&new_peer_group_id)
                .expect("peer group must exist");
            let old_peer = new_peer_group
                .peers
                .insert(mount.unique_id(), Arc::downgrade(mount));
            debug_assert!(old_peer.is_none());
        }

        new
    }

    pub(super) fn unregister_mount_propagation(&mut self, mount: &Mount, old: MountPropagation) {
        if let Some(old_master_id) = old.master_id() {
            let old_master_group = self
                .peer_groups
                .get_mut(&old_master_id)
                .expect("master peer group must exist");
            let old_slave = old_master_group.slaves.remove(&mount.unique_id());
            debug_assert!(old_slave.is_some());
        }

        if old.peer_group_id().is_some() {
            self.remove_peer(mount.unique_id(), &old);
        }
    }

    fn direct_propagation_receivers<'a>(
        &'a self,
        source_parent: &Mount,
    ) -> impl Iterator<Item = Arc<Mount>> + 'a {
        let source_parent_id = source_parent.unique_id();
        let peer_group_id = source_parent.propagation().peer_group_id();
        let peers = peer_group_id
            .into_iter()
            .flat_map(|peer_group_id| self.group_peers_of(peer_group_id))
            .filter(move |peer| peer.unique_id() != source_parent_id);
        let slaves = peer_group_id
            .into_iter()
            .flat_map(|peer_group_id| self.slave_mounts_of(peer_group_id));

        peers.chain(slaves)
    }
}

#[derive(Default)]
pub(super) struct PendingPropagationChanges {
    created_peer_groups: BTreeMap<UniqueMountId, PeerGroup>,
    propagation_ops: Vec<MountPropagationOp>,
}

impl PendingPropagationChanges {
    pub(super) fn make_shared_for_clone(
        &mut self,
        source_mount: &Mount,
    ) -> Result<MountPropagation> {
        let current_propagation = source_mount.propagation();
        if current_propagation.is_shared() {
            return Ok(current_propagation);
        }

        if let Some(peer_group) = self.created_peer_groups.get(&source_mount.unique_id()) {
            return Ok(current_propagation.make_shared(peer_group.id));
        }

        let source_mount = source_mount.this();
        let peer_group = PeerGroup::new()?;
        let new_propagation = current_propagation.make_shared(peer_group.id);
        self.add_peer_group(source_mount.unique_id(), peer_group);
        self.add_set_op(source_mount, new_propagation);
        Ok(new_propagation)
    }

    pub(super) fn add_make_slave_op(&mut self, mount: Arc<Mount>) {
        self.propagation_ops
            .push(MountPropagationOp::MakeSlave { mount });
    }

    pub(super) fn add_set_op(&mut self, mount: Arc<Mount>, new_propagation: MountPropagation) {
        self.propagation_ops.push(MountPropagationOp::Set {
            mount,
            propagation: new_propagation,
        });
    }

    pub(super) fn add_register_op(&mut self, created_mount: Arc<Mount>) {
        self.propagation_ops
            .push(MountPropagationOp::Register(created_mount));
    }

    pub(super) fn commit(mut self, topology: &mut MountTopology) {
        for (_, peer_group) in core::mem::take(&mut self.created_peer_groups) {
            topology.insert_peer_group(peer_group);
        }

        for operation in self.propagation_ops.drain(..) {
            operation.commit(topology);
        }
    }

    pub(super) fn add_peer_group(
        &mut self,
        mount_id: UniqueMountId,
        created_peer_group: PeerGroup,
    ) {
        let old_peer_group = self
            .created_peer_groups
            .insert(mount_id, created_peer_group);
        debug_assert!(old_peer_group.is_none());
    }
}

enum MountPropagationOp {
    Set {
        mount: Arc<Mount>,
        propagation: MountPropagation,
    },
    MakeSlave {
        mount: Arc<Mount>,
    },
    Register(Arc<Mount>),
}

impl MountPropagationOp {
    fn commit(self, topology: &mut MountTopology) {
        match self {
            Self::Set { mount, propagation } => {
                mount.publish_propagation(propagation, topology);
            }
            Self::MakeSlave { mount } => {
                let current_propagation = mount.propagation();
                mount.publish_propagation(current_propagation.make_slave(), topology);
            }
            Self::Register(mount) => topology.register_mount_propagation(&mount),
        }
    }
}

pub(super) struct PeerGroup {
    id: PeerGroupId,
    peers: BTreeMap<UniqueMountId, Weak<Mount>>,
    slaves: BTreeMap<UniqueMountId, Weak<Mount>>,
}

impl PeerGroup {
    pub(super) fn id(&self) -> PeerGroupId {
        self.id
    }

    pub(super) fn new() -> Result<Self> {
        let id = PEER_GROUP_ID_ALLOCATOR
            .lock()
            .alloc()
            .ok_or_else(|| errno!(ENOMEM, "peer group ID pool exhausted"))?;

        Ok(Self {
            id,
            peers: BTreeMap::new(),
            slaves: BTreeMap::new(),
        })
    }
}

impl Drop for PeerGroup {
    fn drop(&mut self) {
        PEER_GROUP_ID_ALLOCATOR.lock().free(self.id);
    }
}

const PEER_GROUP_ID_MIN: PeerGroupId = 1;

const PEER_GROUP_ID_MAX: PeerGroupId = i32::MAX as PeerGroupId;

static PEER_GROUP_ID_ALLOCATOR: SpinLock<SparseIdAlloc> =
    SpinLock::new(SparseIdAlloc::new(PEER_GROUP_ID_MIN, PEER_GROUP_ID_MAX));
