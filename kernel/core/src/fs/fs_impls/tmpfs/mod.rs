//! Temporary file system (tmpfs) based on ramfs.

pub(super) use fs::TmpFs;
use fs::TmpFsType;

mod fs;

pub(super) fn init() {
    crate::fs::vfs::registry::register(&TmpFsType).unwrap();
}
