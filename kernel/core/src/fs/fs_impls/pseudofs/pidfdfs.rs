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
    let _ = PidfdFs::mount_node();
}

pub(crate) struct PidfdFs {
    _private: (),
}

impl PidfdFs {
    pub(crate) fn singleton() -> &'static Arc<NaivePseudoFs> {
        static PIDFDFS: Once<Arc<NaivePseudoFs>> = Once::new();

        NaivePseudoFs::singleton(&PIDFDFS, "pidfdfs", PIDFDFS_MAGIC)
    }

    pub(crate) fn new_path(name_fn: fn(&dyn Inode) -> String) -> Path {
        Path::new_pseudo(
            Self::mount_node().clone(),
            Self::shared_inode().clone(),
            name_fn,
        )
    }

    pub(crate) fn mount_node() -> &'static Arc<Mount> {
        static PIDFDFS_MOUNT: Once<Arc<Mount>> = Once::new();

        PIDFDFS_MOUNT.call_once(|| Mount::new_pseudo(Self::singleton().clone()).unwrap())
    }

    pub(crate) fn shared_inode() -> &'static Arc<dyn Inode> {
        static SHARED_INODE: Once<Arc<dyn Inode>> = Once::new();

        SHARED_INODE.call_once(|| {
            let pidfd_inode = Self::singleton().alloc_inode(
                PseudoInodeType::Pidfd,
                mkmod!(u+rwx),
                Uid::new_root(),
                Gid::new_root(),
            );

            Arc::new(pidfd_inode)
        })
    }
}

const PIDFDFS_MAGIC: u64 = 0x50494446;
