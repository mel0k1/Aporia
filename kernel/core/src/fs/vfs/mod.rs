//! Virtual File System (VFS) layer.
//!
//! The VFS provides a unified abstraction over different file system implementations,
//! serving as the bridge between system calls and concrete file systems.

mod fs_apis;
pub(crate) mod notify;
pub mod path;
pub(crate) mod range_lock;

pub use fs_apis::inode;
pub(crate) use fs_apis::{file_system, inode_ext, registry, xattr};

pub(super) fn init() {
    fs_apis::init();
    path::init();
}
