use alloc::{collections::btree_map::Entry, format};

use keyable_arc::KeyableArc;

use crate::prelude::*;

#[derive(Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct AbstractHandle(KeyableArc<[u8]>);

impl AbstractHandle {
    fn new(name: Arc<[u8]>) -> Self {
        Self(KeyableArc::from(name))
    }

    pub(crate) fn name(&self) -> Arc<[u8]> {
        self.0.clone().into()
    }
}

impl Drop for AbstractHandle {
    fn drop(&mut self) {
        HANDLE_TABLE.remove(self.name());
    }
}

static HANDLE_TABLE: HandleTable = HandleTable::new();

struct HandleTable {
    handles: RwLock<BTreeMap<Arc<[u8]>, Weak<AbstractHandle>>>,
}

impl HandleTable {
    const fn new() -> Self {
        Self {
            handles: RwLock::new(BTreeMap::new()),
        }
    }

    fn create(&self, name: Arc<[u8]>) -> Option<Arc<AbstractHandle>> {
        let mut handles = self.handles.write();

        let mut entry = handles.entry(name.clone());

        if let Entry::Occupied(ref occupied) = entry
            && occupied.get().strong_count() > 0
        {
            return None;
        }

        let new_handle = Arc::new(AbstractHandle::new(name));
        let weak_handle = Arc::downgrade(&new_handle);

        match entry {
            Entry::Occupied(ref mut occupied) => {
                occupied.insert(weak_handle);
            }
            Entry::Vacant(vacant) => {
                vacant.insert(weak_handle);
            }
        }

        Some(new_handle)
    }

    fn remove(&self, name: Arc<[u8]>) {
        let mut handles = self.handles.write();

        let Entry::Occupied(occupied) = handles.entry(name) else {
            return;
        };

        if occupied.get().strong_count() == 0 {
            occupied.remove();
        }
    }

    fn lookup(&self, name: &[u8]) -> Option<Arc<AbstractHandle>> {
        let handles = self.handles.read();

        handles.get(name).and_then(Weak::upgrade)
    }

    fn alloc_ephemeral(&self) -> Option<Arc<AbstractHandle>> {
        (0..(1 << 20))
            .map(|num| format!("{:05x}", num))
            .map(|name| Arc::from(name.as_bytes()))
            .filter_map(|name| self.create(name))
            .next()
    }
}

pub(crate) fn create_abstract_name(name: Arc<[u8]>) -> Result<Arc<AbstractHandle>> {
    HANDLE_TABLE
        .create(name)
        .ok_or_else(|| errno!(EADDRINUSE, "the abstract name is already in use"))
}

pub(crate) fn alloc_ephemeral_abstract_name() -> Result<Arc<AbstractHandle>> {
    HANDLE_TABLE
        .alloc_ephemeral()
        .ok_or_else(|| errno!(ENOSPC, "no ephemeral abstract name is available"))
}

pub(crate) fn lookup_abstract_name(name: &[u8]) -> Result<Arc<AbstractHandle>> {
    HANDLE_TABLE
        .lookup(name)
        .ok_or_else(|| errno!(ECONNREFUSED, "the abstract name does not exist"))
}
