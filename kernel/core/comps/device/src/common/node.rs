use alloc::{
    boxed::Box,
    collections::BTreeMap,
    string::{String, ToString},
    sync::{Arc, Weak},
};
use core::fmt::Write;

use aster_systree::{
    BranchNodeFields, SymlinkNodeFields, SysAttrSet, SysAttrSetBuilder, SysBranchNode, SysNode,
    SysNodeId, SysNodeType, SysObj, SysPerms, inherit_sys_symlink_node,
};
use aster_util::printer::VmPrinter;
use ostd::{
    mm::{VmReader, VmWriter},
    sync::Mutex,
};
use spin::Once;

use crate::common::{Error, Result, SysStr, attr::read_text};

pub(crate) trait SysTreeEdit: Send + Sync {
    fn attach_child(&self, child: Arc<dyn SysObj>) -> Result<()>;

    fn detach_child(&self, name: &str) -> Result<Arc<dyn SysObj>>;

    fn has_children(&self) -> bool;

    fn tree_path(&self) -> String;
}

pub(crate) fn add_link(dir: &dyn SysTreeEdit, name: &str, target_path: &str) -> Result<()> {
    let target = aster_systree::relative_path(&dir.tree_path(), target_path);
    let link = SymlinkNode::new(SysStr::from(name.to_string()), target);
    dir.attach_child(link)
}

pub(crate) fn remove_link(dir: &dyn SysTreeEdit, name: &str) {
    let _ = dir.detach_child(name);
}

pub(crate) struct GlueDirs {
    dirs: Mutex<BTreeMap<SysStr, Arc<Dir>>>,
}

impl GlueDirs {
    pub(crate) const fn new() -> Self {
        Self {
            dirs: Mutex::new(BTreeMap::new()),
        }
    }

    pub(crate) fn attach_into(
        &self,
        name: &str,
        owner: &dyn SysTreeEdit,
        child: Arc<dyn SysObj>,
    ) -> Result<Arc<Dir>> {
        let mut dirs = self.dirs.lock();
        if let Some(dir) = dirs.get(name) {
            dir.attach_child(child)?;
            return Ok(dir.clone());
        }
        let dir = Dir::new(SysStr::from(name.to_string()));
        owner.attach_child(dir.clone())?;
        if let Err(e) = dir.attach_child(child) {
            let _ = owner.detach_child(name);
            return Err(e);
        }
        dirs.insert(SysStr::from(name.to_string()), dir.clone());
        Ok(dir)
    }

    pub(crate) fn drop_if_empty(&self, name: &str, owner: &dyn SysTreeEdit) {
        let mut dirs = self.dirs.lock();
        if let Some(dir) = dirs.get(name)
            && !dir.has_children()
        {
            let _ = owner.detach_child(name);
            dirs.remove(name);
        }
    }
}

pub(crate) trait DirAttrOps: Send + Sync + 'static {
    fn show(&self, _name: &str, _w: &mut dyn Write) -> Result<()> {
        Err(Error::NotFound)
    }

    fn store(&self, _name: &str, _value: &str) -> Result<()> {
        Err(Error::NotFound)
    }
}

pub(crate) struct Dir {
    fields: BranchNodeFields<dyn SysObj, Self>,
    ops: Once<Box<dyn DirAttrOps>>,
}

impl Dir {
    pub(crate) fn new(name: SysStr) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Dir {
            fields: BranchNodeFields::new(name, SysAttrSet::new_empty(), weak_self.clone()),
            ops: Once::new(),
        })
    }

    pub(crate) fn with_attrs(
        name: SysStr,
        attrs: &[(&'static str, SysPerms)],
    ) -> Result<Arc<Self>> {
        let mut builder = SysAttrSetBuilder::new();
        for (attr_name, perms) in attrs {
            builder.add(SysStr::from(*attr_name), *perms);
        }
        let attr_set = builder.build()?;
        Ok(Arc::new_cyclic(|weak_self| Dir {
            fields: BranchNodeFields::new(name, attr_set, weak_self.clone()),
            ops: Once::new(),
        }))
    }

    pub(crate) fn set_ops(&self, ops: Box<dyn DirAttrOps>) {
        self.ops.call_once(|| ops);
    }
}

impl SysTreeEdit for Dir {
    fn attach_child(&self, child: Arc<dyn SysObj>) -> Result<()> {
        self.fields.add_child(child).map_err(Error::from)
    }

    fn detach_child(&self, name: &str) -> Result<Arc<dyn SysObj>> {
        self.fields.remove_child(name).map_err(Error::from)
    }

    fn has_children(&self) -> bool {
        !self.fields.children_ref().read().is_empty()
    }

    fn tree_path(&self) -> String {
        String::from(SysObj::path(self).as_ref())
    }
}

impl SysObj for Dir {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn cast_to_node(&self) -> Option<Arc<dyn SysNode>> {
        self.fields
            .weak_self()
            .upgrade()
            .map(|dir| dir as Arc<dyn SysNode>)
    }

    fn cast_to_branch(&self) -> Option<Arc<dyn SysBranchNode>> {
        self.fields
            .weak_self()
            .upgrade()
            .map(|dir| dir as Arc<dyn SysBranchNode>)
    }

    fn id(&self) -> &SysNodeId {
        self.fields.id()
    }

    fn type_(&self) -> SysNodeType {
        SysNodeType::Branch
    }

    fn name(&self) -> &SysStr {
        self.fields.name()
    }

    fn init_parent(&self, parent: Weak<dyn SysBranchNode>) {
        self.fields.init_parent(parent);
    }

    fn parent(&self) -> Option<Arc<dyn SysBranchNode>> {
        self.fields.parent()
    }
}

impl SysNode for Dir {
    fn node_attrs(&self) -> Arc<SysAttrSet> {
        self.fields.attr_set().clone()
    }

    fn is_attr_absent(&self, _name: &str) -> bool {
        false
    }

    fn read_attr(&self, name: &str, writer: &mut VmWriter) -> aster_systree::Result<usize> {
        self.read_attr_at(name, 0, writer)
    }

    fn write_attr_at(
        &self,
        name: &str,
        _offset: usize,
        reader: &mut VmReader,
    ) -> aster_systree::Result<usize> {
        self.write_attr(name, reader)
    }

    fn write_attr(&self, name: &str, reader: &mut VmReader) -> aster_systree::Result<usize> {
        let ops = self.ops.get().ok_or(aster_systree::Error::NotFound)?;
        let (text, len) = read_text(reader)?;
        ops.store(name, &text)?;
        Ok(len)
    }

    fn read_attr_at(
        &self,
        name: &str,
        offset: usize,
        writer: &mut VmWriter,
    ) -> aster_systree::Result<usize> {
        let ops = self.ops.get().ok_or(aster_systree::Error::NotFound)?;
        let mut printer = VmPrinter::new_skip(writer, offset);
        ops.show(name, &mut printer)?;
        Ok(printer.bytes_written())
    }

    fn perms(&self) -> SysPerms {
        SysPerms::DEFAULT_RW_PERMS
    }
}

impl SysBranchNode for Dir {
    fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&Arc<dyn SysObj>>)) {
        self.fields.visit_child_with(name, f);
    }

    fn visit_children_with(
        &self,
        min_id: u64,
        f: &mut dyn for<'a> FnMut(&'a Arc<dyn SysObj>) -> Option<()>,
    ) {
        self.fields.visit_children_with(min_id, f);
    }

    fn child(&self, name: &str) -> Option<Arc<dyn SysObj>> {
        self.fields.child(name)
    }
}

impl core::fmt::Debug for Dir {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Dir")
            .field("name", self.fields.name())
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub(crate) struct SymlinkNode {
    fields: SymlinkNodeFields<Self>,
}

impl SymlinkNode {
    pub(crate) fn new(name: SysStr, target: String) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| {
            let fields = SymlinkNodeFields::new(name, target, weak_self.clone());
            SymlinkNode { fields }
        })
    }
}

inherit_sys_symlink_node!(SymlinkNode, fields);
