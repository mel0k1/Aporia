use alloc::sync::{Arc, Weak};

use inherit_methods_macro::inherit_methods;

use super::{
    Result,
    attr::SysAttrSet,
    node::{SysBranchNode, SysObj},
};
use crate::{BranchNodeFields, SysPerms, inherit_sys_branch_node};

#[derive(Debug)]
pub struct SysTree<Root: SysBranchNode> {
    root: Arc<Root>,
}

impl SysTree<RootNode> {
    pub(crate) fn new() -> Self {
        let attr_set = SysAttrSet::new_empty();

        let root_node = Arc::new_cyclic(|weak_self| {
            let fields = BranchNodeFields::new_root(attr_set, weak_self.clone());
            RootNode { fields }
        });

        Self { root: root_node }
    }
}

impl<Root: SysBranchNode> SysTree<Root> {
    pub fn new_with_root(root: Arc<Root>) -> Result<Self> {
        if !root.is_root() {
            return Err(crate::Error::InternalError(
                "The provided root is not a root node",
            ));
        }

        Ok(Self { root })
    }

    pub fn root(&self) -> &Arc<Root> {
        &self.root
    }
}

#[derive(Debug)]
pub struct RootNode {
    fields: BranchNodeFields<dyn SysObj, Self>,
}

#[inherit_methods(from = "self.fields")]
impl RootNode {
    pub fn add_child(&self, new_child: Arc<dyn SysObj>) -> Result<()>;
}

inherit_sys_branch_node!(RootNode, fields, {
    fn is_root(&self) -> bool {
        true
    }

    fn init_parent(&self, _parent: Weak<dyn SysBranchNode>) {}

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RO_PERMS
    }
});
