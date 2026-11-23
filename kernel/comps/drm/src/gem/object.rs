use alloc::{boxed::Box, collections::BTreeMap, sync::Arc};
use core::fmt::Debug;

use aster_core::{fs::file::MappedObject, prelude::*};
use ostd::{mm::PAGE_SIZE, sync::Mutex};
use spin::Once;

use crate::gem::mmap_offset::DrmMmapOffset;

#[derive(Debug)]
pub struct DrmGemObject {
    size: usize,
    mmap_offset: Once<DrmMmapOffset>,
    mmap_access_refs_by_client: Mutex<BTreeMap<u64, usize>>,
    backend: Arc<dyn DrmGemObjectBackend>,
}

impl DrmGemObject {
    pub fn new(size: usize, backend: Arc<dyn DrmGemObjectBackend>) -> Result<Self> {
        if size == 0 {
            return Err(errno!(EINVAL, "the GEM object size must not be zero"));
        }
        if !size.is_multiple_of(PAGE_SIZE) {
            return Err(errno!(EINVAL, "the GEM object size is not page-aligned"));
        }

        Ok(Self {
            size,
            mmap_offset: Once::new(),
            mmap_access_refs_by_client: Mutex::new(BTreeMap::new()),
            backend,
        })
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        let end = offset.checked_add(buf.len()).ok_or(Errno::EOVERFLOW)?;
        if end > self.size {
            return Err(errno!(EINVAL, "the GEM read exceeds the object"));
        }

        self.backend.read_bytes(offset, buf)
    }

    pub(super) fn mmap_offset(&self) -> Option<DrmMmapOffset> {
        self.mmap_offset.get().copied()
    }

    pub(crate) fn allow_mmap(&self, client_id: u64) {
        let mut access_refs = self.mmap_access_refs_by_client.lock();
        let count = access_refs.entry(client_id).or_insert(0);
        *count += 1;
    }

    pub(crate) fn revoke_mmap(&self, client_id: u64) {
        let mut access_refs = self.mmap_access_refs_by_client.lock();
        let Some(count) = access_refs.get_mut(&client_id) else {
            return;
        };

        *count -= 1;
        if *count == 0 {
            access_refs.remove(&client_id);
        }
    }

    pub(super) fn is_mmap_allowed(&self, client_id: u64) -> bool {
        self.mmap_access_refs_by_client
            .lock()
            .contains_key(&client_id)
    }

    pub(super) fn set_mmap_offset(&self, offset: DrmMmapOffset) {
        self.mmap_offset.call_once(move || offset);
    }

    pub(super) fn create_mapped_object(
        &self,
        offset: usize,
        size: usize,
    ) -> Result<Box<dyn MappedObject>> {
        self.backend.create_mapped_object(offset, size)
    }
}

pub trait DrmGemObjectBackend: Debug + Send + Sync {
    fn read_bytes(&self, _offset: usize, _buf: &mut [u8]) -> Result<()>;

    fn create_mapped_object(&self, offset: usize, size: usize) -> Result<Box<dyn MappedObject>>;
}
