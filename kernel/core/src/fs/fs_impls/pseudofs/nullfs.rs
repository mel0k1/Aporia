use spin::Once;

use super::NaivePseudoFs;
use crate::prelude::*;

pub(in crate::fs) struct NullFs;

impl NullFs {
    pub(in crate::fs) fn singleton() -> &'static Arc<NaivePseudoFs> {
        static NULLFS: Once<Arc<NaivePseudoFs>> = Once::new();

        NaivePseudoFs::singleton(&NULLFS, "nullfs", NULLFS_MAGIC)
    }
}

const NULLFS_MAGIC: u64 = 0;
