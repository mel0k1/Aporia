//! Inode cache for `virtiofs`.

#![short_vis_path::add(virtiofs)]

use aster_fuse::{EntryReply, FuseNodeId};
use aster_virtio::device::filesystem::device::AttrVersion;
use hashbrown::HashMap;

use super::{VirtioFs, VirtioFsInode};
use crate::prelude::*;

/// In-memory inode cache keyed by FUSE node ID.
pub(in virtiofs) struct InodeCache {
    inodes: RwMutex<HashMap<FuseNodeId, Weak<VirtioFsInode>>>,
}

impl InodeCache {
    pub(in virtiofs) fn new(root: &Arc<VirtioFsInode>) -> Self {
        let inodes = RwMutex::new(HashMap::from_iter([(root.nodeid(), Arc::downgrade(root))]));
        Self { inodes }
    }

    /// Looks up an inode by FUSE node ID and validates its generation.
    pub(in virtiofs) fn lookup_inode(
        &self,
        lookup_reply: EntryReply,
        request_attr_version: AttrVersion,
        fs: &Arc<VirtioFs>,
    ) -> Result<Arc<VirtioFsInode>> {
        let nodeid = lookup_reply.nodeid();

        if let Some(inode) = self
            .inodes
            .read()
            .get(&nodeid)
            .and_then(Weak::upgrade)
            .filter(|inode| inode.generation() == lookup_reply.generation())
        {
            inode.update_from_entry_reply(&lookup_reply, request_attr_version)?;
            return Ok(inode);
        }

        let mut inode_cache = self.inodes.write();
        if let Some(inode) = inode_cache
            .get(&nodeid)
            .and_then(Weak::upgrade)
            .filter(|inode| inode.generation() == lookup_reply.generation())
        {
            drop(inode_cache);

            inode.update_from_entry_reply(&lookup_reply, request_attr_version)?;
            return Ok(inode);
        }

        let inode = VirtioFsInode::new_from_entry_reply(lookup_reply, fs);
        inode_cache.insert(nodeid, Arc::downgrade(&inode));

        Ok(inode)
    }

    /// Inserts an inode by FUSE node ID.
    pub(in virtiofs) fn insert_inode(&self, inode: &Arc<VirtioFsInode>) {
        self.inodes
            .write()
            .insert(inode.nodeid(), Arc::downgrade(inode));
    }

    pub(in virtiofs) fn remove_inode(&self, inode: &VirtioFsInode) -> Option<Weak<VirtioFsInode>> {
        let nodeid = inode.nodeid();
        let mut inodes = self.inodes.write();

        if inodes
            .get(&nodeid)
            .is_some_and(|cached_inode| cached_inode.ptr_eq(&inode.weak_self))
        {
            return inodes.remove(&nodeid);
        }

        None
    }
}
