use alloc::boxed::Box;

use crate::kms::objects::KmsObjectId;

#[derive(Debug)]
pub(crate) struct DrmPropertyBlob {
    id: KmsObjectId,
    data: Box<[u8]>,
}

impl DrmPropertyBlob {
    pub(crate) fn new(id: KmsObjectId, data: Box<[u8]>) -> Self {
        Self { id, data }
    }

    pub(crate) fn id(&self) -> KmsObjectId {
        self.id
    }

    pub(crate) fn data(&self) -> &[u8] {
        &self.data
    }

    pub(crate) fn len(&self) -> usize {
        self.data.len()
    }
}
