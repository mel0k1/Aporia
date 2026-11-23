use alloc::{
    sync::{Arc, Weak},
    vec::Vec,
};
use core::ops::Deref;

use ostd::sync::{Mutex, RwMutex};

use crate::{
    bus::{Bus, BusHandle, DriverHandle},
    common::{
        AnyDevice, DeclaredParts, DevNode, DeviceBase, DeviceBuilder, DeviceInternals, Result,
        Subsystem, SysStr,
        attr::{Attr, TyErasedAttr},
        impl_device_node,
    },
    uevent::UeventVars,
};

pub struct BusDevice<B: Bus> {
    base: DeviceBase,
    bus: Arc<BusHandle<B>>,
    payload: B::Device,
    declared: DeclaredParts<Self>,
    driver: RwMutex<Option<Arc<DriverHandle<B>>>>,
    bind_lock: Mutex<()>,
    weak: Weak<Self>,
}

pub type BusDeviceBuilder<B> = DeviceBuilder<Arc<BusHandle<B>>, <B as Bus>::Device, BusDevice<B>>;

impl<B: Bus> BusDevice<B> {
    pub fn builder(
        bus: &Arc<BusHandle<B>>,
        name: impl Into<SysStr>,
        payload: B::Device,
    ) -> BusDeviceBuilder<B> {
        DeviceBuilder::new(bus.clone(), name.into(), payload)
    }

    pub fn bus(&self) -> &Arc<BusHandle<B>> {
        &self.bus
    }

    pub fn payload(&self) -> &B::Device {
        &self.payload
    }

    pub fn driver(&self) -> Option<Arc<DriverHandle<B>>> {
        self.driver.read().clone()
    }

    pub(crate) fn bind_lock(&self) -> &Mutex<()> {
        &self.bind_lock
    }

    pub(crate) fn take_driver(&self) -> Option<Arc<DriverHandle<B>>> {
        self.driver.write().take()
    }

    pub(crate) fn set_driver(&self, driver: Arc<DriverHandle<B>>) {
        *self.driver.write() = Some(driver);
    }

    pub(crate) fn this(&self) -> Arc<Self> {
        self.weak
            .upgrade()
            .expect("a device is only reachable through an `Arc`")
    }

    pub(crate) fn add_driver_attrs(&self, driver: &Arc<DriverHandle<B>>) -> Result<()> {
        self.base.attrs().add(TyErasedAttr::from_driver(driver))
    }

    pub(crate) fn remove_attrs(&self, attrs: &[Attr<Self>]) {
        let names: Vec<&'static str> = attrs.iter().map(Attr::name).collect();
        self.base.attrs().remove(&names);
    }
}

impl<B: Bus> BusDeviceBuilder<B> {
    pub fn build(self) -> Arc<BusDevice<B>> {
        let declared = self.declared_parts();
        Arc::new_cyclic(|weak: &Weak<BusDevice<B>>| {
            let weak_self: Weak<dyn AnyDevice> = weak.clone();
            BusDevice {
                base: DeviceBase::new(self.name, self.parent, self.devnum, weak_self),
                bus: self.handle,
                payload: self.payload,
                declared,
                driver: RwMutex::new(None),
                bind_lock: Mutex::new(()),
                weak: weak.clone(),
            }
        })
    }
}

impl<B: Bus> AnyDevice for BusDevice<B> {
    fn subsystem(&self) -> Subsystem {
        Subsystem::bus(self.bus.clone())
    }
}

impl<B: Bus> Deref for BusDevice<B> {
    type Target = B::Device;

    fn deref(&self) -> &B::Device {
        &self.payload
    }
}

impl<B: Bus> DeviceInternals for BusDevice<B> {
    fn base(&self) -> &DeviceBase {
        &self.base
    }

    fn attr_groups(&self) -> Vec<TyErasedAttr> {
        self.declared.attr_groups(self.bus.bus().dev_attrs())
    }

    fn devnode_override(&self) -> Option<DevNode> {
        self.declared.type_devnode(self)
    }

    fn append_uevent_vars(&self, vars: &mut UeventVars) -> Result<()> {
        if let Some(name) = self.declared.type_name() {
            vars.add("DEVTYPE", name)?;
        }
        if let Some(driver) = self.driver() {
            vars.add("DRIVER", driver.name())?;
        }

        self.bus.bus().append_uevent_vars(self, vars)?;
        self.declared.append_uevent_vars(self, vars)
    }
}

impl<B: Bus> core::fmt::Debug for BusDevice<B> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BusDevice")
            .field("bus", &B::NAME)
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl_device_node!(BusDevice<B: Bus>);
