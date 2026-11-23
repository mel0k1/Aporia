use alloc::{
    sync::{Arc, Weak},
    vec::Vec,
};

use super::{AnyDevice, DevNode, DeviceBase, DeviceInternals, impl_device_node};
use crate::common::{Result, Subsystem, SysStr, attr::TyErasedAttr};

pub struct BareDevice {
    base: DeviceBase,
}

impl BareDevice {
    pub fn new_root(name: impl Into<SysStr>) -> Arc<Self> {
        Self::new(name, None)
    }

    pub fn with_parent(name: impl Into<SysStr>, parent: Arc<dyn AnyDevice>) -> Arc<Self> {
        Self::new(name, Some(parent))
    }

    fn new(name: impl Into<SysStr>, parent: Option<Arc<dyn AnyDevice>>) -> Arc<Self> {
        Arc::new_cyclic(|weak: &Weak<BareDevice>| {
            let weak_self: Weak<dyn AnyDevice> = weak.clone();
            BareDevice {
                base: DeviceBase::new(name.into(), parent, None, weak_self),
            }
        })
    }
}

impl AnyDevice for BareDevice {
    fn subsystem(&self) -> Subsystem {
        Subsystem::bare()
    }
}

impl DeviceInternals for BareDevice {
    fn base(&self) -> &DeviceBase {
        &self.base
    }

    fn attr_groups(&self) -> Vec<TyErasedAttr> {
        Vec::new()
    }

    fn devnode_override(&self) -> Option<DevNode> {
        None
    }

    fn append_uevent_vars(&self, _vars: &mut crate::uevent::UeventVars) -> Result<()> {
        Ok(())
    }
}

impl core::fmt::Debug for BareDevice {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BareDevice")
            .field("base", &self.base)
            .finish()
    }
}

impl_device_node!(BareDevice);
