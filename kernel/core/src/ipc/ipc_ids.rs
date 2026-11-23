use aster_util::ranged_integer::RangedU32;
use id_alloc::IdAlloc;

use crate::prelude::*;

pub(crate) type IpcId = RangedU32<1, { i32::MAX as u32 }>;

pub(super) struct IpcIds<T> {
    objects: RwMutex<BTreeMap<IpcId, T>>,
    id_allocator: SpinLock<IdAlloc>,
}

impl<T> IpcIds<T> {
    pub(super) fn new(max_id: IpcId) -> Self {
        let mut id_allocator = IdAlloc::with_capacity(max_id.get() as usize + 1);
        id_allocator.alloc_specific(0).unwrap();

        Self {
            objects: RwMutex::new(BTreeMap::new()),
            id_allocator: SpinLock::new(id_allocator),
        }
    }

    pub(super) fn with<R, F>(&self, id: IpcId, op: F) -> Result<R, IdNotExistError>
    where
        F: FnOnce(&T) -> R,
    {
        let objects = self.objects.read();

        let Some(object) = objects.get(&id) else {
            return Err(IdNotExistError);
        };

        Ok(op(object))
    }

    pub(super) fn remove<F>(&self, id: IpcId, may_remove: F) -> Result<()>
    where
        F: FnOnce(&T) -> Result<()>,
    {
        use alloc::collections::btree_map::Entry;

        let mut objects = self.objects.write();

        let Entry::Occupied(entry) = objects.entry(id) else {
            return Err(errno!(EINVAL, "the ID does not exist"));
        };

        may_remove(entry.get())?;
        entry.remove();

        self.id_allocator.lock().free(id.get() as usize);

        Ok(())
    }

    pub(super) fn insert_auto<F>(&self, new_object_fn: F) -> Result<IpcId>
    where
        F: FnOnce(IpcId) -> Result<T>,
    {
        let mut objects = self.objects.write();

        let Some(id) = self
            .id_allocator
            .lock()
            .alloc()
            .map(|id| IpcId::new(id as u32))
        else {
            return Err(errno!(ENOSPC, "all IDs are exhausted"));
        };

        let object = match new_object_fn(id) {
            Ok(object) => object,
            Err(err) => {
                self.id_allocator.lock().free(id.get() as usize);
                return Err(err);
            }
        };
        objects.insert(id, object);

        Ok(id)
    }

    pub(super) fn insert_at<F>(&self, id: IpcId, new_object_fn: F) -> Result<()>
    where
        F: FnOnce(IpcId) -> Result<T>,
    {
        let mut objects = self.objects.write();

        if self
            .id_allocator
            .lock()
            .alloc_specific(id.get() as usize)
            .is_none()
        {
            return Err(errno!(EEXIST, "the ID already exists"));
        }

        let object = match new_object_fn(id) {
            Ok(object) => object,
            Err(err) => {
                self.id_allocator.lock().free(id.get() as usize);
                return Err(err);
            }
        };
        objects.insert(id, object);

        Ok(())
    }
}

#[derive(Debug)]
pub(super) struct IdNotExistError;

impl From<IdNotExistError> for Error {
    #[track_caller]
    fn from(_value: IdNotExistError) -> Self {
        errno!(EINVAL, "the ID does not exist")
    }
}
