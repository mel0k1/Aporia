use alloc::{
    collections::BTreeMap,
    string::String,
    sync::{Arc, Weak},
    vec::Vec,
};

use inherit_methods_macro::inherit_methods;
use ostd::{
    mm::{VmReader, VmWriter},
    sync::RwMutex,
};
use spin::Once;

use super::{
    Error, Result, SysStr,
    attr::SysAttrSet,
    node::{SysNodeId, SysObj},
};
use crate::{SysBranchNode, SysNode, SysSymlink};

pub fn is_valid_name(name: &str) -> bool {
    !name.is_empty() && !matches!(name, "." | "..") && !name.contains(['/', '\0'])
}

pub fn relative_path(from_dir: &str, to: &str) -> String {
    let from: Vec<&str> = from_dir.split('/').filter(|s| !s.is_empty()).collect();
    let to: Vec<&str> = to.split('/').filter(|s| !s.is_empty()).collect();
    let Some((last, to_parent)) = to.split_last() else {
        return String::from("/");
    };
    let common = from
        .iter()
        .zip(to_parent.iter())
        .take_while(|(a, b)| a == b)
        .count();

    let mut result = String::new();
    for _ in common..from.len() {
        result.push_str("../");
    }
    for component in &to_parent[common..] {
        result.push_str(component);
        result.push('/');
    }
    result.push_str(last);
    result
}

#[derive(Debug)]
pub struct ObjFields<T: SysObj + ?Sized> {
    id: SysNodeId,
    name: SysStr,
    parent: Once<Weak<dyn SysBranchNode>>,
    weak_self: Weak<T>,
}

impl<T: SysObj + ?Sized> ObjFields<T> {
    pub fn new(name: SysStr, weak_self: Weak<T>) -> Self {
        assert!(is_valid_name(&name), "invalid SysTree node name");
        Self {
            name,
            ..Self::new_root(weak_self)
        }
    }

    pub fn obj_field(&self) -> &ObjFields<T> {
        self
    }

    pub fn id(&self) -> &SysNodeId {
        &self.id
    }

    pub fn name(&self) -> &SysStr {
        &self.name
    }

    pub fn init_parent(&self, parent: Weak<dyn SysBranchNode>) {
        self.parent.call_once(|| parent);
    }

    pub fn parent(&self) -> Option<Arc<dyn SysBranchNode>> {
        self.parent
            .get()
            .and_then(|weak_parent| weak_parent.upgrade())
    }

    pub fn weak_self(&self) -> &Weak<T> {
        &self.weak_self
    }

    fn new_root(weak_self: Weak<T>) -> Self {
        Self {
            id: SysNodeId::new(),
            name: SysStr::from(""),
            parent: Once::new(),
            weak_self,
        }
    }
}

#[derive(Debug)]
pub struct NormalNodeFields<T: SysNode> {
    base: ObjFields<T>,
    attr_set: Arc<SysAttrSet>,
}

#[inherit_methods(from = "self.base")]
impl<T: SysNode> NormalNodeFields<T> {
    pub fn new(name: SysStr, attr_set: SysAttrSet, weak_self: Weak<T>) -> Self {
        Self {
            base: ObjFields::new(name, weak_self),
            attr_set: Arc::new(attr_set),
        }
    }

    pub fn obj_field(&self) -> &ObjFields<T>;

    pub fn id(&self) -> &SysNodeId;

    pub fn name(&self) -> &SysStr;

    pub fn init_parent(&self, parent: Weak<dyn SysBranchNode>);

    pub fn parent(&self) -> Option<Arc<dyn SysBranchNode>>;

    pub fn weak_self(&self) -> &Weak<T>;

    pub fn attr_set(&self) -> &Arc<SysAttrSet> {
        &self.attr_set
    }
}

#[derive(Debug)]
pub struct AttrLessBranchNodeFields<C: SysObj + ?Sized, T: SysBranchNode> {
    base: ObjFields<T>,
    pub children: RwMutex<BTreeMap<SysStr, Arc<C>>>,
}

#[inherit_methods(from = "self.base")]
impl<C: SysObj + ?Sized, T: SysBranchNode> AttrLessBranchNodeFields<C, T> {
    pub fn new(name: SysStr, weak_self: Weak<T>) -> Self {
        Self {
            base: ObjFields::new(name, weak_self),
            children: RwMutex::new(BTreeMap::new()),
        }
    }

    pub fn obj_field(&self) -> &ObjFields<T>;

    pub fn id(&self) -> &SysNodeId;

    pub fn name(&self) -> &SysStr;

    pub fn init_parent(&self, parent: Weak<dyn SysBranchNode>);

    pub fn parent(&self) -> Option<Arc<dyn SysBranchNode>>;

    pub fn weak_self(&self) -> &Weak<T>;

    pub fn contains(&self, child_name: &str) -> bool {
        let children = self.children.read();
        children.contains_key(child_name)
    }

    pub fn add_child(&self, new_child: Arc<C>) -> Result<()> {
        let mut children = self.children.write();
        let name = new_child.name();
        if children.contains_key(name) {
            return Err(Error::AlreadyExists);
        }

        new_child.init_parent(self.weak_self().clone());
        children.insert(name.clone(), new_child);

        Ok(())
    }

    pub fn remove_child(&self, child_name: &str) -> Result<Arc<C>> {
        let mut children = self.children.write();
        children.remove(child_name).ok_or(Error::NotFound)
    }

    pub fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&Arc<C>>)) {
        let children_guard = self.children.read();
        f(children_guard.get(name))
    }

    pub fn visit_children_with(&self, min_id: u64, f: &mut dyn FnMut(&Arc<C>) -> Option<()>) {
        let children_guard = self.children.read();
        for child_arc in children_guard.values() {
            if child_arc.id().as_u64() < min_id {
                continue;
            }

            if f(child_arc).is_none() {
                break;
            }
        }
    }

    pub fn child(&self, name: &str) -> Option<Arc<C>> {
        let children = self.children.read();
        children.get(name).cloned()
    }

    pub fn children_ref(&self) -> &RwMutex<BTreeMap<SysStr, Arc<C>>> {
        &self.children
    }

    pub fn attr_set(&self) -> &Arc<SysAttrSet> {
        SysAttrSet::empty()
    }

    fn new_root(weak_self: Weak<T>) -> Self {
        Self {
            base: ObjFields::new_root(weak_self),
            children: RwMutex::new(BTreeMap::new()),
        }
    }
}

#[derive(Debug)]
pub struct BranchNodeFields<C: SysObj + ?Sized, T: SysBranchNode> {
    base: AttrLessBranchNodeFields<C, T>,
    attr_set: Arc<SysAttrSet>,
}

#[inherit_methods(from = "self.base")]
impl<C: SysObj + ?Sized, T: SysBranchNode> BranchNodeFields<C, T> {
    pub fn new(name: SysStr, attr_set: SysAttrSet, weak_self: Weak<T>) -> Self {
        Self {
            base: AttrLessBranchNodeFields::new(name, weak_self),
            attr_set: Arc::new(attr_set),
        }
    }

    pub fn obj_field(&self) -> &ObjFields<T>;

    pub fn id(&self) -> &SysNodeId;

    pub fn name(&self) -> &SysStr;

    pub fn init_parent(&self, parent: Weak<dyn SysBranchNode>);

    pub fn parent(&self) -> Option<Arc<dyn SysBranchNode>>;

    pub fn weak_self(&self) -> &Weak<T>;

    pub fn contains(&self, child_name: &str) -> bool;

    pub fn add_child(&self, new_child: Arc<C>) -> Result<()>;

    pub fn remove_child(&self, child_name: &str) -> Result<Arc<C>>;

    pub fn visit_child_with(&self, name: &str, f: &mut dyn FnMut(Option<&Arc<C>>));

    pub fn visit_children_with(&self, min_id: u64, f: &mut dyn FnMut(&Arc<C>) -> Option<()>);

    pub fn child(&self, name: &str) -> Option<Arc<C>>;

    pub fn children_ref(&self) -> &RwMutex<BTreeMap<SysStr, Arc<C>>>;

    pub fn attr_set(&self) -> &Arc<SysAttrSet> {
        &self.attr_set
    }

    pub(crate) fn new_root(attr_set: SysAttrSet, weak_self: Weak<T>) -> Self {
        Self {
            base: AttrLessBranchNodeFields::new_root(weak_self),
            attr_set: Arc::new(attr_set),
        }
    }
}

#[derive(Debug)]
pub struct SymlinkNodeFields<T: SysSymlink> {
    base: ObjFields<T>,
    target_path: String,
}

#[inherit_methods(from = "self.base")]
impl<T: SysSymlink> SymlinkNodeFields<T> {
    pub fn new(name: SysStr, target_path: String, weak_self: Weak<T>) -> Self {
        Self {
            base: ObjFields::new(name, weak_self),
            target_path,
        }
    }

    pub fn obj_field(&self) -> &ObjFields<T>;

    pub fn id(&self) -> &SysNodeId;

    pub fn name(&self) -> &SysStr;

    pub fn init_parent(&self, parent: Weak<dyn SysBranchNode>);

    pub fn parent(&self) -> Option<Arc<dyn SysBranchNode>>;

    pub fn weak_self(&self) -> &Weak<T>;

    pub fn target_path(&self) -> &str {
        &self.target_path
    }
}

#[doc(hidden)]
#[macro_export]
macro_rules! _inner_impl_sys_node {
    ($struct_name:ident, $field:ident, $helper_trait:ty) => {
        impl $crate::SysNode for $struct_name {
            fn node_attrs(&self) -> alloc::sync::Arc<$crate::SysAttrSet> {
                self.$field.attr_set().clone()
            }

            fn is_attr_absent(&self, name: &str) -> bool {
                <_ as $helper_trait>::is_attr_absent(self, name)
            }

            fn read_attr(
                &self,
                name: &str,
                writer: &mut ostd::mm::VmWriter,
            ) -> $crate::Result<usize> {
                <_ as $helper_trait>::read_attr(self, name, writer)
            }

            fn write_attr(
                &self,
                name: &str,
                reader: &mut ostd::mm::VmReader,
            ) -> $crate::Result<usize> {
                <_ as $helper_trait>::write_attr(self, name, reader)
            }

            fn read_attr_at(
                &self,
                name: &str,
                offset: usize,
                writer: &mut ostd::mm::VmWriter,
            ) -> $crate::Result<usize> {
                <_ as $helper_trait>::read_attr_at(self, name, offset, writer)
            }

            fn write_attr_at(
                &self,
                name: &str,
                offset: usize,
                reader: &mut ostd::mm::VmReader,
            ) -> $crate::Result<usize> {
                <_ as $helper_trait>::write_attr_at(self, name, offset, reader)
            }

            fn perms(&self) -> $crate::SysPerms {
                <_ as $helper_trait>::perms(self)
            }
        }
    };
}

#[doc(hidden)]
pub trait _InheritSysLeafNode<T: SysNode> {
    fn field(&self) -> &ObjFields<T>;

    fn is_root(&self) -> bool {
        false
    }

    fn init_parent(&self, parent: Weak<dyn SysBranchNode>) {
        self.field().init_parent(parent);
    }

    fn is_attr_absent(&self, _name: &str) -> bool {
        false
    }

    fn read_attr(&self, name: &str, writer: &mut VmWriter) -> Result<usize> {
        self.read_attr_at(name, 0, writer)
    }

    fn write_attr(&self, _name: &str, _reader: &mut VmReader) -> Result<usize> {
        Err(Error::AttributeError)
    }

    fn read_attr_at(&self, _name: &str, _offset: usize, _writer: &mut VmWriter) -> Result<usize> {
        Err(Error::AttributeError)
    }

    fn write_attr_at(&self, name: &str, _offset: usize, reader: &mut VmReader) -> Result<usize> {
        self.write_attr(name, reader)
    }

    fn perms(&self) -> crate::SysPerms;
}

#[macro_export]
macro_rules! inherit_sys_leaf_node {
    ($struct_name:ident, $field:ident, {$($fn_override:item)*}) => {
        impl $crate::_InheritSysLeafNode<$struct_name> for $struct_name {
            fn field(&self) -> &$crate::ObjFields<$struct_name> {
                &self.$field.obj_field()
            }

            $($fn_override)*
        }

        impl $crate::SysObj for $struct_name {
            fn as_any(&self) -> &dyn core::any::Any {
                self
            }

            fn cast_to_node(&self) -> Option<alloc::sync::Arc<dyn $crate::SysNode>> {
                <_ as $crate::_InheritSysLeafNode<$struct_name>>::field(self)
                    .weak_self()
                    .upgrade()
                    .map(|arc| arc as alloc::sync::Arc<dyn $crate::SysNode>)
            }

            fn type_(&self) -> $crate::SysNodeType {
                $crate::SysNodeType::Leaf
            }

            fn id(&self) -> &$crate::SysNodeId {
                self.$field.id()
            }

            fn name(&self) -> &$crate::SysStr {
                self.$field.name()
            }

            fn is_root(&self) -> bool {
                <_ as $crate::_InheritSysLeafNode<$struct_name>>::is_root(self)
            }

            fn init_parent(&self, parent: alloc::sync::Weak<dyn $crate::SysBranchNode>) {
                <_ as $crate::_InheritSysLeafNode<$struct_name>>::init_parent(self, parent)
            }

            fn parent(&self) -> Option<alloc::sync::Arc<dyn $crate::SysBranchNode>> {
                self.$field.parent()
            }
        }

        $crate::_inner_impl_sys_node!(
            $struct_name,
            $field,
            $crate::_InheritSysLeafNode<$struct_name>
        );
    };
}

#[doc(hidden)]
pub trait _InheritSysBranchNode<T: SysBranchNode> {
    fn field(&self) -> &ObjFields<T>;

    fn is_root(&self) -> bool {
        false
    }

    fn init_parent(&self, parent: Weak<dyn SysBranchNode>) {
        self.field().init_parent(parent);
    }

    fn is_attr_absent(&self, _name: &str) -> bool {
        false
    }

    fn read_attr(&self, name: &str, writer: &mut VmWriter) -> Result<usize> {
        self.read_attr_at(name, 0, writer)
    }

    fn write_attr(&self, _name: &str, _reader: &mut VmReader) -> Result<usize> {
        Err(Error::AttributeError)
    }

    fn read_attr_at(&self, _name: &str, _offset: usize, _writer: &mut VmWriter) -> Result<usize> {
        Err(Error::AttributeError)
    }

    fn write_attr_at(&self, name: &str, _offset: usize, reader: &mut VmReader) -> Result<usize> {
        self.write_attr(name, reader)
    }

    fn perms(&self) -> crate::SysPerms;

    fn create_child(&self, _name: &str) -> Result<Arc<dyn SysObj>> {
        Err(Error::PermissionDenied)
    }
}

#[macro_export]
macro_rules! inherit_sys_branch_node {
    ($struct_name:ident, $field:ident, {$($fn_override:item)*}) => {
        impl $crate::_InheritSysBranchNode<$struct_name> for $struct_name {
            fn field(&self) -> &$crate::ObjFields<$struct_name> {
                &self.$field.obj_field()
            }

            $($fn_override)*
        }

        impl $crate::SysObj for $struct_name {
            fn as_any(&self) -> &dyn core::any::Any {
                self
            }

            fn cast_to_node(&self) -> Option<alloc::sync::Arc<dyn $crate::SysNode>> {
                <_ as $crate::_InheritSysBranchNode<$struct_name>>::field(self)
                    .weak_self()
                    .upgrade()
                    .map(|arc| arc as alloc::sync::Arc<dyn $crate::SysNode>)
            }

            fn cast_to_branch(&self) -> Option<alloc::sync::Arc<dyn $crate::SysBranchNode>> {
                <_ as $crate::_InheritSysBranchNode<$struct_name>>::field(self)
                    .weak_self()
                    .upgrade()
                    .map(|arc| arc as alloc::sync::Arc<dyn $crate::SysBranchNode>)
            }

            fn type_(&self) -> $crate::SysNodeType {
                $crate::SysNodeType::Branch
            }

            fn id(&self) -> &$crate::SysNodeId {
                self.$field.id()
            }

            fn name(&self) -> &$crate::SysStr {
                self.$field.name()
            }

            fn is_root(&self) -> bool {
                <_ as $crate::_InheritSysBranchNode<$struct_name>>::is_root(self)
            }

            fn init_parent(&self, parent: alloc::sync::Weak<dyn $crate::SysBranchNode>) {
                <_ as $crate::_InheritSysBranchNode<$struct_name>>::init_parent(self, parent)
            }

            fn parent(&self) -> Option<alloc::sync::Arc<dyn $crate::SysBranchNode>> {
                self.$field.parent()
            }
        }

        $crate::_inner_impl_sys_node!(
            $struct_name,
            $field,
            $crate::_InheritSysBranchNode<$struct_name>
        );

        impl $crate::SysBranchNode for $struct_name {
            fn visit_child_with(
                &self,
                name: &str,
                f: &mut dyn FnMut(Option<&alloc::sync::Arc<dyn $crate::SysObj>>)
            ) {
                let children_guard = self.$field.children_ref().read();
                let child = children_guard
                    .get(name)
                    .map(|child| child.clone() as alloc::sync::Arc<dyn $crate::SysObj>);

                f(child.as_ref())
            }

            fn visit_children_with(
                &self,
                min_id: u64,
                f: &mut dyn for<'a> FnMut(&'a alloc::sync::Arc<dyn $crate::SysObj>) -> Option<()>,
            ) {
                let children_guard = self.$field.children_ref().read();
                for child_arc in children_guard.values() {
                    if child_arc.id().as_u64() < min_id {
                        continue;
                    }

                    let child = child_arc.clone() as alloc::sync::Arc<dyn $crate::SysObj>;
                    if f(&child).is_none() {
                        break;
                    }
                }
            }

            fn child(&self, name: &str) -> Option<Arc<dyn $crate::SysObj>> {
                self.$field
                    .child(name)
                    .map(|child| child as Arc<dyn $crate::SysObj>)
            }

            fn create_child(
                &self,
                name: &str
            ) -> $crate::Result<alloc::sync::Arc<dyn $crate::SysObj>> {
                <_ as $crate::_InheritSysBranchNode<Self>>::create_child(self, name)
            }

            fn remove_child(
                &self,
                name: &str
            ) -> $crate::Result<alloc::sync::Arc<dyn $crate::SysObj>> {
                self.$field
                    .remove_child(name)
                    .map(|child| child as Arc<dyn $crate::SysObj>)
            }
        }
    };
}

#[doc(hidden)]
pub trait _InheritSysSymlinkNode<T: SysSymlink> {
    fn field(&self) -> &ObjFields<T>;

    fn is_root(&self) -> bool {
        false
    }

    fn init_parent(&self, parent: Weak<dyn SysBranchNode>) {
        self.field().init_parent(parent);
    }
}

#[macro_export]
macro_rules! inherit_sys_symlink_node {
    ($struct_name:ident, $field:ident, {$($fn_override:item)*}) => {
        impl $crate::_InheritSysSymlinkNode<$struct_name> for $struct_name {
            fn field(&self) -> &$crate::ObjFields<$struct_name> {
                &self.$field.obj_field()
            }

            $($fn_override)*
        }

        impl $crate::SysObj for $struct_name {
            fn as_any(&self) -> &dyn core::any::Any {
                self
            }

            fn cast_to_symlink(&self) -> Option<Arc<dyn $crate::SysSymlink>> {
                <_ as $crate::_InheritSysSymlinkNode<$struct_name>>::field(self)
                    .weak_self()
                    .upgrade()
                    .map(|arc| arc as Arc<dyn $crate::SysSymlink>)
            }

            fn type_(&self) -> $crate::SysNodeType {
                $crate::SysNodeType::Symlink
            }

            fn id(&self) -> &$crate::SysNodeId {
                self.$field.id()
            }

            fn name(&self) -> &$crate::SysStr {
                self.$field.name()
            }

            fn is_root(&self) -> bool {
                <_ as $crate::_InheritSysSymlinkNode<$struct_name>>::is_root(self)
            }

            fn init_parent(&self, parent: alloc::sync::Weak<dyn $crate::SysBranchNode>) {
                <_ as $crate::_InheritSysSymlinkNode<$struct_name>>::init_parent(self, parent)
            }

            fn parent(&self) -> Option<alloc::sync::Arc<dyn $crate::SysBranchNode>> {
                self.$field.parent()
            }
        }

        impl $crate::SysSymlink for $struct_name {
            fn target_path(&self) -> &str {
                self.$field.target_path()
            }
        }
    };

    ($struct_name:ident, $field:ident) => {
        $crate::inherit_sys_symlink_node!($struct_name, $field, { });
    };
}

#[derive(Debug)]
pub struct EmptyNode {
    fields: NormalNodeFields<Self>,
}

impl EmptyNode {
    pub fn new(name: SysStr) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| {
            let attr_set = SysAttrSet::new_empty();
            let fields = NormalNodeFields::new(name, attr_set, weak_self.clone());
            Self { fields }
        })
    }
}

inherit_sys_leaf_node!(EmptyNode, fields, {
    fn perms(&self) -> crate::SysPerms {
        crate::SysPerms::DEFAULT_RO_PERMS
    }

    fn read_attr(&self, _name: &str, _writer: &mut VmWriter) -> Result<usize> {
        Err(Error::NotFound)
    }

    fn write_attr(&self, _name: &str, _reader: &mut VmReader) -> Result<usize> {
        Err(Error::NotFound)
    }
});
