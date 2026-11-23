mod device;
mod driver;

use alloc::{
    boxed::Box,
    sync::{Arc, Weak},
    vec::Vec,
};
use core::{
    fmt::Write,
    sync::atomic::{AtomicBool, Ordering},
};

use aster_systree::{SysObj, SysPerms};
use ostd::sync::Mutex;

pub use self::{
    device::{BusDevice, BusDeviceBuilder},
    driver::{Driver, DriverHandle},
};
use crate::{
    common::{
        AnyDevice, DeviceInternals, Error, Result, SysStr,
        attr::Attr,
        node::{Dir, DirAttrOps, SysTreeEdit, add_link, remove_link},
        registry,
        subsystem::SubsystemOps,
    },
    uevent::{self, UeventAction, UeventVars},
};

pub trait Bus: Sized + Send + Sync + 'static {
    const NAME: &'static str;

    type Device: Send + Sync + 'static;

    type MatchData: ?Sized + Send + Sync + 'static;

    fn matches(&self, dev: &Self::Device, data: &Self::MatchData) -> bool;

    fn dev_attrs(&self) -> &'static [Attr<BusDevice<Self>>];

    fn append_uevent_vars(&self, dev: &BusDevice<Self>, vars: &mut UeventVars) -> Result<()>;
}

pub fn register<B: Bus>(bus: B) -> Result<Arc<BusHandle<B>>> {
    let dir = Dir::with_attrs(
        SysStr::from(B::NAME),
        &[
            ("drivers_autoprobe", SysPerms::DEFAULT_RW_ATTR_PERMS),
            ("drivers_probe", SysPerms::OWNER_W),
        ],
    )?;
    let devices_dir = Dir::new(SysStr::from("devices"));
    let drivers_dir = Dir::new(SysStr::from("drivers"));
    dir.attach_child(devices_dir.clone())?;
    dir.attach_child(drivers_dir.clone())?;

    let handle = Arc::new_cyclic(|weak| BusHandle {
        bus,
        dir: dir.clone(),
        devices_dir,
        drivers_dir,
        devices: Mutex::new(Vec::new()),
        drivers: Mutex::new(Vec::new()),
        autoprobe: AtomicBool::new(true),
        weak: weak.clone(),
    });
    dir.set_ops(Box::new(BusDirOps {
        bus: handle.weak.clone(),
    }));
    registry::get().bus_root().attach_child(dir)?;
    registry::get().keep_subsystem(handle.clone());
    Ok(handle)
}

pub struct BusHandle<B: Bus> {
    bus: B,
    dir: Arc<Dir>,
    devices_dir: Arc<Dir>,
    drivers_dir: Arc<Dir>,
    devices: Mutex<Vec<Arc<BusDevice<B>>>>,
    drivers: Mutex<Vec<Arc<DriverHandle<B>>>>,
    autoprobe: AtomicBool,
    weak: Weak<Self>,
}

impl<B: Bus> BusHandle<B> {
    pub fn register_driver(&self, driver: Box<dyn Driver<B>>) -> Result<Arc<DriverHandle<B>>> {
        if self
            .drivers
            .lock()
            .iter()
            .any(|d| d.name() == driver.name())
        {
            return Err(Error::NameConflict);
        }

        let handle = DriverHandle::new(driver, self.weak.clone())?;
        self.drivers_dir.attach_child(handle.dir().clone())?;
        self.drivers.lock().push(handle.clone());

        if self.autoprobe.load(Ordering::Relaxed) {
            for dev in self.devices() {
                if dev.driver().is_none() {
                    let _ = self.try_bind(&dev, &handle);
                }
            }
        }

        Ok(handle)
    }

    pub fn unregister_driver(&self, driver: &Arc<DriverHandle<B>>) -> Result<()> {
        let removed = {
            let mut drivers = self.drivers.lock();
            let before = drivers.len();
            drivers.retain(|d| !Arc::ptr_eq(d, driver));
            drivers.len() != before
        };
        if !removed {
            return Err(Error::NotFound);
        }

        let devices = driver.state().lock().set_unregistered()?;
        for dev in devices {
            let _ = self.unbind_from(&dev, driver);
        }
        let _ = self.drivers_dir.detach_child(driver.name());

        Ok(())
    }

    pub fn unbind(&self, dev: &Arc<BusDevice<B>>) -> Result<()> {
        self.unbind_inner(dev, None)
    }

    pub(crate) fn bind(
        &self,
        dev: &Arc<BusDevice<B>>,
        driver: &Arc<DriverHandle<B>>,
    ) -> Result<()> {
        if dev.driver().is_some() {
            return Err(Error::AlreadyBound);
        }
        self.try_bind(dev, driver)
    }

    fn probe(&self, dev: &Arc<BusDevice<B>>) -> Result<()> {
        if dev.driver().is_some() {
            return Err(Error::AlreadyBound);
        }
        for driver in self.drivers() {
            if self.try_bind(dev, &driver).is_ok() {
                return Ok(());
            }
        }
        Err(Error::NoDriver)
    }

    fn unbind_from(&self, dev: &Arc<BusDevice<B>>, expected: &Arc<DriverHandle<B>>) -> Result<()> {
        self.unbind_inner(dev, Some(expected))
    }

    fn try_bind(&self, dev: &Arc<BusDevice<B>>, driver: &Arc<DriverHandle<B>>) -> Result<()> {
        if !self.bus.matches(dev.payload(), driver.match_data()) {
            return Err(Error::NoDriver);
        }

        let _guard = dev.bind_lock().lock();
        if !dev.base().is_added() {
            return Err(Error::NotAdded);
        }
        if dev.driver().is_some() {
            return Err(Error::AlreadyBound);
        }

        driver.state().lock().start_binding(dev)?;
        if let Err(error) = self.bind_inner(dev, driver) {
            driver.state().lock().abort_binding(dev);
            return Err(error);
        }
        driver.state().lock().finish_binding(dev);

        if let Err(error) =
            uevent::emit_unchecked(dev.as_ref(), UeventAction::Bind, UeventVars::new())
        {
            ostd::warn!("cannot emit bind for {}: {error}", dev.name());
        }

        Ok(())
    }

    fn bind_inner(&self, dev: &Arc<BusDevice<B>>, driver: &Arc<DriverHandle<B>>) -> Result<()> {
        add_link(
            driver.dir().as_ref(),
            dev.base().name(),
            &dev.base().tree_path(),
        )?;
        if let Err(e) = add_link(dev.base(), "driver", &SysObj::path(driver.dir().as_ref())) {
            remove_link(driver.dir().as_ref(), dev.base().name());
            return Err(e);
        }

        if let Err(e) = driver.on_probe(dev) {
            driver::remove_driver_links(dev, driver);
            return Err(match e {
                Error::NoDriver => Error::NoDriver,
                _ => Error::ProbeFailed,
            });
        }
        if let Err(e) = dev.add_driver_attrs(driver) {
            driver::remove_driver_links(dev, driver);
            driver.on_release(dev);
            return Err(e);
        }

        dev.set_driver(driver.clone());

        Ok(())
    }

    fn unbind_inner(
        &self,
        dev: &Arc<BusDevice<B>>,
        expected: Option<&Arc<DriverHandle<B>>>,
    ) -> Result<()> {
        let _guard = dev.bind_lock().lock();

        if let Some(expected) = expected
            && !dev.driver().is_some_and(|d| Arc::ptr_eq(&d, expected))
        {
            return Err(Error::NotBound);
        }
        let driver = dev.driver().ok_or(Error::NotBound)?;

        driver::remove_driver_links(dev, &driver);
        dev.remove_attrs(driver.dev_attrs());
        driver.on_release(dev);
        dev.take_driver();
        driver.state().lock().forget_bound(dev);

        if let Err(error) =
            uevent::emit_unchecked(dev.as_ref(), UeventAction::Unbind, UeventVars::new())
        {
            ostd::warn!("cannot emit unbind for {}: {error}", dev.name());
        }

        Ok(())
    }

    fn is_autoprobe_enabled(&self) -> bool {
        self.autoprobe.load(Ordering::Relaxed)
    }

    fn set_autoprobe(&self, on: bool) {
        self.autoprobe.store(on, Ordering::Relaxed);
    }

    fn bus(&self) -> &B {
        &self.bus
    }

    fn devices(&self) -> Vec<Arc<BusDevice<B>>> {
        self.devices.lock().clone()
    }

    fn drivers(&self) -> Vec<Arc<DriverHandle<B>>> {
        self.drivers.lock().clone()
    }

    fn find_device(&self, name: &str) -> Option<Arc<BusDevice<B>>> {
        self.devices
            .lock()
            .iter()
            .find(|d| d.base().name() == name)
            .cloned()
    }
}

impl<B: Bus> SubsystemOps for BusHandle<B> {
    fn name(&self) -> &'static str {
        B::NAME
    }

    fn dir(&self) -> Arc<Dir> {
        self.dir.clone()
    }

    fn index_dir(&self) -> Arc<Dir> {
        self.devices_dir.clone()
    }

    fn keeps_glue_dir(&self) -> bool {
        false
    }

    fn on_added(&self, dev: &Arc<dyn AnyDevice>) {
        let dev = dev
            .as_any()
            .downcast_ref::<BusDevice<B>>()
            .expect("a bus device reports its own bus as its subsystem");
        let dev = dev.this();

        self.devices.lock().push(dev.clone());
        if self.autoprobe.load(Ordering::Relaxed) {
            let _ = self.probe(&dev);
        }
    }

    fn on_removed(&self, dev: &Arc<dyn AnyDevice>) {
        let dev = dev
            .as_any()
            .downcast_ref::<BusDevice<B>>()
            .expect("a bus device reports its own bus as its subsystem");
        let dev = dev.this();

        let was_present = {
            let mut devices = self.devices.lock();
            let before = devices.len();
            devices.retain(|d| !Arc::ptr_eq(d, &dev));
            devices.len() != before
        };
        if was_present {
            let _ = self.unbind(&dev);
        }
    }
}

impl<B: Bus> core::fmt::Debug for BusHandle<B> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BusHandle").field("name", &B::NAME).finish()
    }
}

struct BusDirOps<B: Bus> {
    bus: Weak<BusHandle<B>>,
}

impl<B: Bus> DirAttrOps for BusDirOps<B> {
    fn show(&self, name: &str, w: &mut dyn Write) -> Result<()> {
        let bus = self.bus.upgrade().ok_or(Error::NotFound)?;

        match name {
            "drivers_autoprobe" => {
                writeln!(w, "{}", if bus.is_autoprobe_enabled() { 1 } else { 0 })?;
                Ok(())
            }
            _ => Err(Error::NotFound),
        }
    }

    fn store(&self, name: &str, value: &str) -> Result<()> {
        let bus = self.bus.upgrade().ok_or(Error::NotFound)?;

        match name {
            "drivers_autoprobe" => match value.trim() {
                "0" => bus.set_autoprobe(false),
                "1" => bus.set_autoprobe(true),
                _ => return Err(Error::InvalidValue),
            },
            "drivers_probe" => {
                let dev = bus.find_device(value.trim()).ok_or(Error::NotFound)?;
                match bus.probe(&dev) {
                    Ok(()) | Err(Error::AlreadyBound) | Err(Error::NoDriver) => {}
                    Err(e) => return Err(e),
                }
            }
            _ => return Err(Error::NotFound),
        }

        Ok(())
    }
}
