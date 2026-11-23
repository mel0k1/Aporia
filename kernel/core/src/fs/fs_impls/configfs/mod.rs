use aster_systree::{EmptyNode, SysBranchNode};
use systree_node::ConfigRootNode;

use crate::{fs::configfs::fs::ConfigFsType, prelude::*};

mod fs;
mod inode;
mod systree_node;
#[cfg(ktest)]
mod test;

pub(super) fn init() {
    let config_kernel_sysnode = EmptyNode::new("config".into());
    super::sysfs::register_kernel_sysnode(config_kernel_sysnode).unwrap();

    crate::fs::vfs::registry::register(&ConfigFsType).unwrap();
}

#[cfg_attr(not(any(ktest, target_arch = "x86_64")), expect(dead_code))]
pub(crate) fn register_subsystem(subsystem: Arc<dyn SysBranchNode>) -> Result<()> {
    ConfigRootNode::singleton().add_child(subsystem)?;

    Ok(())
}

#[expect(dead_code)]
pub(crate) fn unregister_subsystem(name: &str) -> Result<()> {
    ConfigRootNode::singleton().remove_child(name)?;

    Ok(())
}

#[cfg(ktest)]
pub(crate) fn init_for_ktest() {
    aster_systree::init_for_ktest();
    crate::fs::vfs::init();
    super::sysfs::init();
    init();
}
