use alloc::format;

use spin::Once;

use crate::{
    fs::{
        pipe::AnonPipeInode,
        pseudofs::NaivePseudoFs,
        vfs::{
            file_system::FileSystem,
            path::{Mount, Path},
            registry::{FsCreationCtx, FsProperties, FsType},
        },
    },
    prelude::*,
};

pub(super) fn init() {
    let _ = PipeFs::mount_node();
}

pub(in crate::fs) struct PipeFs {
    _private: (),
}

impl PipeFs {
    pub(in crate::fs) fn singleton() -> &'static Arc<NaivePseudoFs> {
        static PIPEFS: Once<Arc<NaivePseudoFs>> = Once::new();

        NaivePseudoFs::singleton(&PIPEFS, "pipefs", PIPEFS_MAGIC)
    }

    pub(in crate::fs) fn new_path(pipe_inode: Arc<AnonPipeInode>) -> Path {
        Path::new_pseudo(Self::mount_node().clone(), pipe_inode, |inode| {
            format!("pipe:[{}]", inode.ino())
        })
    }

    fn mount_node() -> &'static Arc<Mount> {
        static PIPEFS_MOUNT: Once<Arc<Mount>> = Once::new();

        PIPEFS_MOUNT.call_once(|| Mount::new_pseudo(Self::singleton().clone()).unwrap())
    }
}

pub(super) struct PipeFsType;

impl FsType for PipeFsType {
    type Key = ();

    fn name(&self) -> &'static str {
        "pipefs"
    }

    fn properties(&self) -> FsProperties {
        FsProperties::empty()
    }

    fn create(&self, _fs_creation_ctx: &mut FsCreationCtx) -> Result<Arc<dyn FileSystem>> {
        Err(errno!(EINVAL, "pipefs cannot be mounted"))
    }

    fn sysnode(&self) -> Option<Arc<dyn aster_systree::SysNode>> {
        None
    }
}

const PIPEFS_MAGIC: u64 = 0x50495045;
