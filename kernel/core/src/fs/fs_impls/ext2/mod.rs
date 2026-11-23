macro_rules! __log_prefix {
    () => {
        "ext2: "
    };
}

pub(crate) use fs::Ext2;
pub(crate) use inode::{FilePerm, Inode};

pub(in crate::fs) use self::fs_type::EXT2_TYPE;
use crate::fs::vfs::registry;

mod block_group;
mod fs;
mod fs_type;
mod impl_for_vfs;
mod inode;
mod prelude;
mod super_block;
mod utils;
mod xattr;

#[cfg(ktest)]
mod test_utils;

pub(super) fn init() {
    registry::register(&EXT2_TYPE).unwrap();
}
