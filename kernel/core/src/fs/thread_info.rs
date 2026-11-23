use core::sync::atomic::{AtomicU16, Ordering};

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;

use super::vfs::path::PathResolver;
use crate::prelude::*;

pub(crate) struct ThreadFsInfo {
    resolver: RwMutex<PathResolver>,
    umask: AtomicFileCreationMask,
}

impl ThreadFsInfo {
    pub(crate) fn new(path_resolver: PathResolver) -> Self {
        Self {
            resolver: RwMutex::new(path_resolver),
            umask: AtomicFileCreationMask::new(FileCreationMask::default()),
        }
    }

    pub(crate) fn resolver(&self) -> &RwMutex<PathResolver> {
        &self.resolver
    }

    pub(crate) fn umask(&self) -> FileCreationMask {
        self.umask.load(Ordering::Acquire)
    }

    pub(crate) fn swap_umask(&self, new_mask: FileCreationMask) -> FileCreationMask {
        self.umask.swap(new_mask, Ordering::AcqRel)
    }
}

impl Clone for ThreadFsInfo {
    fn clone(&self) -> Self {
        Self {
            resolver: RwMutex::new(self.resolver.read().clone()),
            umask: AtomicFileCreationMask::new(self.umask.load(Ordering::Acquire)),
        }
    }
}

pub(crate) struct FileCreationMask(u16);

impl FileCreationMask {
    const MASK: u16 = 0o777;

    pub(crate) fn get(&self) -> u16 {
        self.0
    }
}

impl Default for FileCreationMask {
    fn default() -> Self {
        Self(0o022)
    }
}

impl TryFrom<u16> for FileCreationMask {
    type Error = Error;

    fn try_from(value: u16) -> Result<Self> {
        if value & !Self::MASK != 0 {
            Err(errno!(EINVAL, "Invalid FileCreationMask.",))
        } else {
            Ok(Self(value))
        }
    }
}

impl From<FileCreationMask> for u16 {
    fn from(value: FileCreationMask) -> Self {
        value.0
    }
}

define_atomic_version_of_integer_like_type!(FileCreationMask, try_from = true, {
    #[derive(Debug)]
    pub(crate) struct AtomicFileCreationMask(AtomicU16);
});
