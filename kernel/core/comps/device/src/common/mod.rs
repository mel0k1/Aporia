pub(crate) mod attr;
mod bare_device;
mod devnum;
mod error;
mod internals;
pub(crate) mod node;
pub(crate) mod registration;
pub(crate) mod registry;
pub(crate) mod subsystem;

use alloc::{
    sync::{Arc, Weak},
    vec::Vec,
};

use aster_systree::SysBranchNode;

pub(crate) use self::internals::{DeviceBase, DeviceInternals};
use self::{
    attr::TyErasedAttr,
    node::{Dir, SysTreeEdit},
};
pub use self::{
    attr::{Attr, ShowFn, StoreFn},
    bare_device::BareDevice,
    devnum::{DEFAULT_DEVNODE_MODE, DevKind, DevNum},
    error::{Error, Result},
    registration::DeviceBuilder,
    subsystem::{Subsystem, SubsystemKind},
};

pub type SysStr = aster_systree::SysStr;

pub trait AnyDevice: SysBranchNode + DeviceInternals {
    fn devnum(&self) -> Option<DevNum> {
        self.base().devnum()
    }

    fn subsystem(&self) -> Subsystem;

    fn to_arc(&self) -> Arc<dyn AnyDevice> {
        self.base()
            .fields()
            .weak_self()
            .upgrade()
            .expect("a device is only reachable through an `Arc`")
    }
}

pub struct DeviceType<D: 'static> {
    pub name: &'static str,
    pub attrs: &'static [Attr<D>],
    pub devnode: Option<fn(&D) -> Option<DevNode>>,
    pub has_device_link: bool,
    pub append_uevent_vars_fn: Option<fn(&D, &mut crate::uevent::UeventVars) -> Result<()>>,
}

impl<D: 'static> DeviceType<D> {
    pub const fn named(name: &'static str) -> Self {
        Self {
            name,
            attrs: &[],
            devnode: None,
            has_device_link: true,
            append_uevent_vars_fn: None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DevNode {
    pub path: Option<SysStr>,
    pub mode: Option<u16>,
}

pub(crate) struct DeclaredParts<D: 'static> {
    dev_type: Option<&'static DeviceType<D>>,
    own_attrs: &'static [Attr<D>],
}

impl<D: AnyDevice> DeclaredParts<D> {
    pub(crate) fn type_name(&self) -> Option<&'static str> {
        self.dev_type.map(|t| t.name)
    }

    pub(crate) fn attr_groups(&self, subsystem_attrs: &[Attr<D>]) -> Vec<TyErasedAttr> {
        let mut attrs = TyErasedAttr::from_typed_slice(subsystem_attrs);
        if let Some(t) = self.dev_type {
            attrs.extend(TyErasedAttr::from_typed_slice(t.attrs));
        }
        attrs.extend(TyErasedAttr::from_typed_slice(self.own_attrs));
        attrs
    }

    pub(crate) fn type_devnode(&self, dev: &D) -> Option<DevNode> {
        self.dev_type.and_then(|t| t.devnode).and_then(|f| f(dev))
    }

    pub(crate) fn has_device_link(&self) -> bool {
        self.dev_type.is_none_or(|t| t.has_device_link)
    }

    pub(crate) fn append_uevent_vars(
        &self,
        dev: &D,
        vars: &mut crate::uevent::UeventVars,
    ) -> Result<()> {
        if let Some(callback_fn) = self.dev_type.and_then(|t| t.append_uevent_vars_fn) {
            callback_fn(dev, vars)?;
        }
        Ok(())
    }
}

#[derive(Debug)]
enum State {
    Initialized,
    Preparing,
    Adding(ChildDevices),
    Added(ChildDevices),
    Removed,
}

type ChildDevices = Vec<Weak<dyn AnyDevice>>;

#[derive(Default)]
struct Links {
    subsystem: bool,
    device: bool,
    index: bool,
    dev_index: bool,
}

enum TreeParent {
    Dir(Weak<Dir>),
    Device(Weak<dyn AnyDevice>),
}

impl TreeParent {
    fn with_edit<R>(&self, f: impl FnOnce(&dyn SysTreeEdit) -> R) -> Option<R> {
        match self {
            TreeParent::Dir(dir) => dir.upgrade().map(|dir| f(dir.as_ref())),
            TreeParent::Device(dev) => dev.upgrade().map(|dev| f(dev.base())),
        }
    }
}

macro_rules! impl_device_node {
    ($ty:ident $(< $p:ident : $bound:path >)?) => {
        impl$(<$p: $bound>)? ::aster_systree::SysObj for $ty$(<$p>)? {
            fn as_any(&self) -> &dyn core::any::Any {
                self
            }

            fn cast_to_node(&self) -> Option<::alloc::sync::Arc<dyn ::aster_systree::SysNode>> {
                self.base()
                    .fields()
                    .weak_self()
                    .upgrade()
                    .map(|d| d as ::alloc::sync::Arc<dyn ::aster_systree::SysNode>)
            }

            fn cast_to_branch(
                &self
            ) -> Option<::alloc::sync::Arc<dyn ::aster_systree::SysBranchNode>> {
                self.base()
                    .fields()
                    .weak_self()
                    .upgrade()
                    .map(|d| d as ::alloc::sync::Arc<dyn ::aster_systree::SysBranchNode>)
            }

            fn id(&self) -> &::aster_systree::SysNodeId {
                self.base().fields().id()
            }

            fn type_(&self) -> ::aster_systree::SysNodeType {
                ::aster_systree::SysNodeType::Branch
            }

            fn name(&self) -> &$crate::common::SysStr {
                self.base().fields().name()
            }

            fn init_parent(&self, parent: ::alloc::sync::Weak<dyn ::aster_systree::SysBranchNode>) {
                self.base().fields().init_parent(parent);
            }

            fn parent(&self) -> Option<::alloc::sync::Arc<dyn ::aster_systree::SysBranchNode>> {
                self.base().fields().parent()
            }
        }

        impl$(<$p: $bound>)? ::aster_systree::SysNode for $ty$(<$p>)? {
            fn node_attrs(&self) -> ::alloc::sync::Arc<::aster_systree::SysAttrSet> {
                self.base().attrs().set()
            }

            fn is_attr_absent(&self, _name: &str) -> bool {
                false
            }

            fn read_attr(
                &self,
                name: &str,
                writer: &mut ::ostd::mm::VmWriter
            ) -> aster_systree::Result<usize> {
                self.read_attr_at(name, 0, writer)
            }

            fn write_attr_at(
                &self,
                name: &str,
                _offset: usize,
                reader: &mut ::ostd::mm::VmReader,
            ) -> aster_systree::Result<usize> {
                self.write_attr(name, reader)
            }

            fn write_attr(
                &self,
                name: &str,
                reader: &mut ::ostd::mm::VmReader
            ) -> aster_systree::Result<usize> {
                if !self.base().is_added() {
                    return Err(aster_systree::Error::IsDead);
                }
                self.base().attrs().store(self, name, reader)
            }

            fn read_attr_at(
                &self,
                name: &str,
                offset: usize,
                writer: &mut ::ostd::mm::VmWriter,
            ) -> aster_systree::Result<usize> {
                if !self.base().is_added() {
                    return Err(aster_systree::Error::IsDead);
                }
                self.base().attrs().show(self, name, offset, writer)
            }

            fn perms(&self) -> ::aster_systree::SysPerms {
                ::aster_systree::SysPerms::DEFAULT_RW_PERMS
            }
        }

        impl$(<$p: $bound>)? ::aster_systree::SysBranchNode for $ty$(<$p>)? {
            fn visit_child_with(
                &self,
                name: &str,
                f: &mut dyn FnMut(Option<&::alloc::sync::Arc<dyn ::aster_systree::SysObj>>)
            ) {
                let children = self.base().children().read();
                f(children.get(name))
            }

            fn visit_children_with(
                &self,
                min_id: u64,
                f: &mut dyn for<'a> FnMut(
                    &'a ::alloc::sync::Arc<dyn ::aster_systree::SysObj>
                ) -> Option<()>,
            ) {
                let children = self.base().children().read();
                for child in children.values() {
                    if child.id().as_u64() < min_id {
                        continue;
                    }
                    if f(child).is_none() {
                        break;
                    }
                }
            }

            fn child(&self, name: &str) -> Option<::alloc::sync::Arc<dyn ::aster_systree::SysObj>> {
                self.base().children().read().get(name).cloned()
            }
        }
    };
}

pub(crate) use impl_device_node;
