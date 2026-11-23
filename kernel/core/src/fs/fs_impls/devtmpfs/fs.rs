use spin::Once;

use crate::{
    fs::{
        tmpfs::TmpFs,
        vfs::{
            file_system::FileSystem,
            inode::RevalidationPolicy,
            registry::{FsCreationCtx, FsProperties, FsType},
        },
    },
    prelude::*,
};

pub(in crate::fs) fn singleton() -> &'static Arc<TmpFs> {
    static SINGLETON: Once<Arc<TmpFs>> = Once::new();

    SINGLETON.call_once(|| {
        TmpFs::new_tmpfs_backing(
            "devtmpfs",
            RevalidationPolicy::REVALIDATE_EXISTS | RevalidationPolicy::REVALIDATE_ABSENT,
        )
    })
}

pub(super) fn init() {
    crate::fs::vfs::registry::register(&DevTmpFsType).unwrap();
}

struct DevTmpFsType;

impl FsType for DevTmpFsType {
    type Key = ();

    fn name(&self) -> &'static str {
        "devtmpfs"
    }

    fn properties(&self) -> FsProperties {
        FsProperties::empty()
    }

    fn create(&self, _fs_creation_ctx: &mut FsCreationCtx) -> Result<Arc<dyn FileSystem>> {
        Ok(singleton().clone())
    }

    fn sysnode(&self) -> Option<Arc<dyn aster_systree::SysNode>> {
        None
    }
}
