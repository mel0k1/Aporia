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
