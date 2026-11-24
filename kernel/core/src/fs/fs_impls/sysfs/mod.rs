mod fs;
mod inode;
mod kernel;
#[cfg(ktest)]
mod test;

use aster_systree::SysNode;
pub(crate) use aster_systree::primary_tree as systree_singleton;
use fs::SysFsType;

use crate::{fs::vfs::registry, prelude::*};

pub(crate) fn init() {
    registry::register(&SysFsType).unwrap();

    kernel::init();
}

/// Registers a new kernel `SysNode`.
pub(crate) fn register_kernel_sysnode(config_obj: Arc<dyn SysNode>) -> Result<()> {
    kernel::register(config_obj)
}

/// Unregisters a kernel `SysNode`.
#[expect(dead_code)]
pub(crate) fn unregister_kernel_sysnode(name: &str) -> Result<()> {
    kernel::unregister(name)
}
