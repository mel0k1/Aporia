mod device;
mod observer;

use alloc::{sync::Arc, vec::Vec};

use ostd::sync::Mutex;

pub use self::{
    device::{ClassDevice, ClassDeviceBuilder},
    observer::ClassObserver,
};
use crate::common::{
    AnyDevice, DevNode, DeviceInternals, Error, Result, SysStr,
    attr::Attr,
    node::{Dir, SysTreeEdit},
    registry,
    subsystem::SubsystemOps,
};

pub trait Class: Sized + Send + Sync + 'static {
    const NAME: &'static str;

    type Device: Send + Sync + 'static;

    fn devnode(&self, _dev: &ClassDevice<Self>) -> Option<DevNode> {
        None
    }

    fn dev_attrs(&self) -> &'static [Attr<ClassDevice<Self>>];

    fn append_uevent_vars(
        &self,
        dev: &ClassDevice<Self>,
        vars: &mut crate::uevent::UeventVars,
    ) -> Result<()>;

    const KEEPS_GLUE_DIR: bool = false;
}

pub fn register<C: Class>(class: C) -> Result<Arc<ClassHandle<C>>> {
    let dir = Dir::new(SysStr::from(C::NAME));
    registry::get().class_root().attach_child(dir.clone())?;

    let handle = Arc::new(ClassHandle {
        class,
        dir,
        devices: Mutex::new(Vec::new()),
        observers: Mutex::new(Vec::new()),
        membership: Mutex::new(()),
    });
    registry::get().keep_subsystem(handle.clone());
    Ok(handle)
}

pub struct ClassHandle<C: Class> {
    class: C,
    dir: Arc<Dir>,
    devices: Mutex<Vec<Arc<ClassDevice<C>>>>,
    observers: Mutex<Vec<Arc<dyn ClassObserver<C>>>>,
    membership: Mutex<()>,
}

impl<C: Class> ClassHandle<C> {
    pub fn register_observer(&self, observer: Arc<dyn ClassObserver<C>>) -> Result<()> {
        let _guard = self.membership.lock();
        {
            let mut observers = self.observers.lock();
            if observers.iter().any(|o| Arc::ptr_eq(o, &observer)) {
                return Err(Error::AlreadyAdded);
            }
            observers.push(observer.clone());
        }
        for dev in self.devices() {
            observer.on_device_added(&dev);
        }
        Ok(())
    }

    pub fn unregister_observer(&self, observer: &Arc<dyn ClassObserver<C>>) -> Result<()> {
        let _guard = self.membership.lock();
        let removed = {
            let mut observers = self.observers.lock();
            let before = observers.len();
            observers.retain(|o| !Arc::ptr_eq(o, observer));
            observers.len() != before
        };
        if !removed {
            return Err(Error::NotFound);
        }
        for dev in self.devices() {
            observer.on_device_removed(&dev);
        }
        Ok(())
    }

    pub fn class(&self) -> &C {
        &self.class
    }

    pub fn devices(&self) -> Vec<Arc<ClassDevice<C>>> {
        self.devices.lock().clone()
    }

    pub fn find_device(&self, name: &str) -> Option<Arc<ClassDevice<C>>> {
        self.devices
            .lock()
            .iter()
            .find(|d| d.base().name() == name)
            .cloned()
    }

    fn observers(&self) -> Vec<Arc<dyn ClassObserver<C>>> {
        self.observers.lock().clone()
    }
}

impl<C: Class> SubsystemOps for ClassHandle<C> {
    fn name(&self) -> &'static str {
        C::NAME
    }

    fn dir(&self) -> Arc<Dir> {
        self.dir.clone()
    }

    fn index_dir(&self) -> Arc<Dir> {
        self.dir.clone()
    }

    fn keeps_glue_dir(&self) -> bool {
        C::KEEPS_GLUE_DIR
    }

    fn on_added(&self, dev: &Arc<dyn AnyDevice>) {
        let dev = dev
            .as_any()
            .downcast_ref::<ClassDevice<C>>()
            .expect("a class device reports its own class as its subsystem");
        let dev = dev.this();

        let _guard = self.membership.lock();
        self.devices.lock().push(dev.clone());
        for observer in self.observers() {
            observer.on_device_added(&dev);
        }
    }

    fn on_removed(&self, dev: &Arc<dyn AnyDevice>) {
        let dev = dev
            .as_any()
            .downcast_ref::<ClassDevice<C>>()
            .expect("a class device reports its own class as its subsystem");
        let dev = dev.this();

        let _guard = self.membership.lock();
        let is_member = self.devices.lock().iter().any(|d| Arc::ptr_eq(d, &dev));
        if !is_member {
            return;
        }
        for observer in self.observers() {
            observer.on_device_removed(&dev);
        }
        self.devices.lock().retain(|d| !Arc::ptr_eq(d, &dev));
    }
}

impl<C: Class> core::fmt::Debug for ClassHandle<C> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ClassHandle")
            .field("name", &C::NAME)
            .finish()
    }
}
