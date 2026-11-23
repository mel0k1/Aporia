#![short_vis_path::add(overlayfs)]

use core::sync::atomic::{AtomicU64, Ordering};

use hashbrown::HashMap;

use crate::{
    fs::fs_impls::overlayfs::inode::{OverlayInode, identity::ObjectRealId},
    prelude::*,
};

#[derive(Debug)]
pub(in overlayfs) struct InodeCache {
    entries: RwMutex<HashMap<ObjectRealId, Weak<OverlayInode>>>,
    misses_since_sweep: AtomicU64,
}

impl InodeCache {
    pub(in overlayfs) fn new() -> Self {
        Self {
            entries: RwMutex::new(HashMap::new()),
            misses_since_sweep: AtomicU64::new(0),
        }
    }

    pub(super) fn occupant(&self, real_id: ObjectRealId) -> Option<Arc<OverlayInode>> {
        let guard = self.entries.read();
        guard.get(&real_id).and_then(Weak::upgrade)
    }

    pub(super) fn register(
        &self,
        real_id: ObjectRealId,
        instance: Arc<OverlayInode>,
    ) -> Arc<OverlayInode> {
        const SWEEP_INTERVAL: u64 = 1024;
        let mut guard = self.entries.write();
        if let Some(occupant) = guard.get(&real_id).and_then(Weak::upgrade) {
            return occupant;
        }
        guard.remove(&real_id);
        let misses = self.misses_since_sweep.fetch_add(1, Ordering::Relaxed) + 1;
        if misses.is_multiple_of(SWEEP_INTERVAL) {
            guard.retain(|_, entry| entry.strong_count() > 0);
        }
        guard.insert(real_id, Arc::downgrade(&instance));
        instance
    }

    pub(super) fn rekey(
        &self,
        old_key: ObjectRealId,
        new_key: ObjectRealId,
        instance: &Arc<OverlayInode>,
    ) {
        let mut guard = self.entries.write();
        let is_own_entry = guard
            .get(&old_key)
            .and_then(Weak::upgrade)
            .is_some_and(|occupant| Arc::ptr_eq(&occupant, instance));
        if !is_own_entry {
            return;
        }
        guard.remove(&old_key);
        guard.insert(new_key, Arc::downgrade(instance));
    }
}
