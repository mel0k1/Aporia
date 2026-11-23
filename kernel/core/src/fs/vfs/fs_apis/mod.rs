pub(crate) mod file_system;
pub mod inode;
pub(crate) mod inode_ext;
pub(crate) mod registry;
pub(crate) mod xattr;

pub(super) fn init() {
    registry::init();
}
