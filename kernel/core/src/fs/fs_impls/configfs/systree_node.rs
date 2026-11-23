use alloc::sync::{Arc, Weak};
use core::fmt::Debug;

use aster_systree::{
    BranchNodeFields, Result, SysAttrSet, SysBranchNode, SysObj, SysPerms, SysStr,
    inherit_sys_branch_node,
};
use inherit_methods_macro::inherit_methods;
use spin::Once;

#[derive(Debug)]
pub(crate) struct ConfigRootNode {
    fields: BranchNodeFields<dyn SysObj, Self>,
}

#[inherit_methods(from = "self.fields")]
impl ConfigRootNode {
    pub(super) fn singleton() -> &'static Arc<ConfigRootNode> {
        static SINGLETON: Once<Arc<ConfigRootNode>> = Once::new();

        SINGLETON.call_once(Self::new)
    }

    fn new() -> Arc<Self> {
        let name = SysStr::from("config");

        let attrs = SysAttrSet::new_empty();
        Arc::new_cyclic(|weak_self| {
            let fields = BranchNodeFields::new(name, attrs, weak_self.clone());
            ConfigRootNode { fields }
        })
    }

    pub(crate) fn add_child(&self, new_child: Arc<dyn SysObj>) -> Result<()>;
}

inherit_sys_branch_node!(ConfigRootNode, fields, {
    fn is_root(&self) -> bool {
        true
    }

    fn init_parent(&self, _parent: Weak<dyn SysBranchNode>) {}

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }
});
