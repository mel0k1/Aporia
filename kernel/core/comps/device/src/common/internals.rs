use alloc::{
    collections::BTreeMap,
    string::String,
    sync::{Arc, Weak},
    vec::Vec,
};

use aster_systree::{ObjFields, SysBranchNode, SysObj};
use ostd::sync::{Mutex, RwMutex};
use spin::Once;

use super::{AnyDevice, DevNode, Links, State, TreeParent};
use crate::{
    common::{
        Error, Result, SysStr,
        attr::{AttrTable, TyErasedAttr},
        devnum::DevNum,
        node::{GlueDirs, SysTreeEdit},
    },
    hooks::DevNodeRequest,
    uevent::UeventVars,
};

pub trait DeviceInternals {
    fn base(&self) -> &DeviceBase;

    fn attr_groups(&self) -> Vec<TyErasedAttr>;

    fn devnode_override(&self) -> Option<DevNode>;

    fn append_uevent_vars(&self, vars: &mut UeventVars) -> Result<()>;

    fn wants_device_link(&self) -> bool {
        true
    }
}

pub struct DeviceBase {
    fields: ObjFields<dyn AnyDevice>,
    tree_parent: Once<TreeParent>,
    children: RwMutex<BTreeMap<SysStr, Arc<dyn SysObj>>>,
    attrs: AttrTable,
    parent: Option<Arc<dyn AnyDevice>>,
    devnum: Option<DevNum>,
    state: Mutex<State>,
    glue_dirs: GlueDirs,
    links: Mutex<Links>,
    devnode: Mutex<Option<DevNodeRequest>>,
}

impl DeviceBase {
    pub(crate) fn name(&self) -> &SysStr {
        self.fields.name()
    }

    pub(crate) fn parent(&self) -> Option<&Arc<dyn AnyDevice>> {
        self.parent.as_ref()
    }

    pub(crate) fn is_added(&self) -> bool {
        matches!(*self.state.lock(), State::Adding(_) | State::Added(_))
    }

    pub(crate) fn new(
        name: SysStr,
        parent: Option<Arc<dyn AnyDevice>>,
        devnum: Option<DevNum>,
        weak_self: Weak<dyn AnyDevice>,
    ) -> Self {
        Self {
            fields: ObjFields::new(name, weak_self),
            tree_parent: Once::new(),
            children: RwMutex::new(BTreeMap::new()),
            attrs: AttrTable::new(),
            parent,
            devnum,
            state: Mutex::new(State::Initialized),
            glue_dirs: GlueDirs::new(),
            links: Mutex::new(Links::default()),
            devnode: Mutex::new(None),
        }
    }

    pub(crate) fn fields(&self) -> &ObjFields<dyn AnyDevice> {
        &self.fields
    }

    pub(crate) fn children(&self) -> &RwMutex<BTreeMap<SysStr, Arc<dyn SysObj>>> {
        &self.children
    }

    pub(crate) fn attrs(&self) -> &AttrTable {
        &self.attrs
    }

    pub(super) fn tree_parent(&self) -> &Once<TreeParent> {
        &self.tree_parent
    }

    pub(super) fn devnum(&self) -> Option<DevNum> {
        self.devnum
    }

    pub(super) fn state(&self) -> &Mutex<State> {
        &self.state
    }

    pub(super) fn glue_dirs(&self) -> &GlueDirs {
        &self.glue_dirs
    }

    pub(super) fn links(&self) -> &Mutex<Links> {
        &self.links
    }

    pub(crate) fn devnode(&self) -> &Mutex<Option<DevNodeRequest>> {
        &self.devnode
    }
}

impl SysTreeEdit for DeviceBase {
    fn attach_child(&self, child: Arc<dyn SysObj>) -> Result<()> {
        let mut children = self.children.write();
        let name = child.name().clone();
        if children.contains_key(&name) {
            return Err(Error::NameConflict);
        }
        let weak: Weak<dyn SysBranchNode> = self.fields.weak_self().clone();
        child.init_parent(weak);
        children.insert(name, child);
        Ok(())
    }

    fn detach_child(&self, name: &str) -> Result<Arc<dyn SysObj>> {
        self.children.write().remove(name).ok_or(Error::NotFound)
    }

    fn has_children(&self) -> bool {
        !self.children.read().is_empty()
    }

    fn tree_path(&self) -> String {
        let Some(parent) = self.fields.parent() else {
            return String::from(self.name().as_ref());
        };
        let mut path = String::from(parent.path().as_ref());
        if !parent.is_root() {
            path.push('/');
        }
        path.push_str(self.name());
        path
    }
}

impl core::fmt::Debug for DeviceBase {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DeviceBase")
            .field("name", self.name())
            .field("devnum", &self.devnum)
            .field("state", &*self.state.lock())
            .finish_non_exhaustive()
    }
}
