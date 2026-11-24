use spin::Once;

use crate::{
    fs::{
        file::mkmod,
        pseudofs::{NaivePseudoFs, PseudoInodeType},
        vfs::{
            inode::Inode,
            path::{Mount, Path},
        },
    },
    prelude::*,
    process::{Gid, Uid},
};

pub(super) fn init() {
    let _ = AnonInodeFs::mount_node();
}

pub(crate) struct AnonInodeFs {
    _private: (),
}

impl AnonInodeFs {
    /// Returns the singleton instance of the anonymous inode file system.
    fn singleton() -> &'static Arc<NaivePseudoFs> {
        static ANON_INODEFS: Once<Arc<NaivePseudoFs>> = Once::new();

        NaivePseudoFs::singleton(&ANON_INODEFS, "anon_inodefs", ANON_INODEFS_MAGIC)
    }

    /// Creates a pseudo `Path` for the shared inode.
    pub(crate) fn new_path(name_fn: fn(&dyn Inode) -> String) -> Path {
        Path::new_pseudo(
            Self::mount_node().clone(),
            Self::shared_inode().clone(),
            name_fn,
        )
    }

    /// Returns the pseudo mount node of the anonymous inode file system.
    pub(crate) fn mount_node() -> &'static Arc<Mount> {
        static ANON_INODEFS_MOUNT: Once<Arc<Mount>> = Once::new();

        ANON_INODEFS_MOUNT.call_once(|| Mount::new_pseudo(Self::singleton().clone()).unwrap())
    }

    /// Returns the shared inode of the anonymous inode file system singleton.
    pub(crate) fn shared_inode() -> &'static Arc<dyn Inode> {
        static SHARED_INODE: Once<Arc<dyn Inode>> = Once::new();

        SHARED_INODE.call_once(|| {
            let shared_inode = Self::singleton().alloc_inode(
                PseudoInodeType::AnonInode,
                mkmod!(u+rw),
                Uid::new_root(),
                Gid::new_root(),
            );

            Arc::new(shared_inode)
        })
    }
}

const ANON_INODEFS_MAGIC: u64 = 0x09041934;
