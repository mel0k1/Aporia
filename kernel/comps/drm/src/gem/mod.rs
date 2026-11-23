use alloc::sync::Arc;
use core::fmt::Debug;

use aster_core::prelude::*;

use crate::gem::object::DrmGemObject;

pub(crate) mod mmap_offset;
pub mod object;
pub mod shmem;

pub trait DrmGemOps: Debug + Send + Sync {
    fn create_dumb(&self, size: usize) -> Result<Arc<DrmGemObject>>;
}
