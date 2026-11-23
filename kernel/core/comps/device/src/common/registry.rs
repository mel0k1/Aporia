use alloc::{sync::Arc, vec::Vec};

use aster_systree::SysObj;
use ostd::sync::Mutex;
use spin::Once;

use crate::common::{
    DevKind, Result, SysStr,
    node::{Dir, GlueDirs, SysTreeEdit},
    subsystem::SubsystemOps,
};

pub(crate) fn get() -> &'static Registry {
    REGISTRY.get().expect("the device model is not initialized")
}

pub(crate) fn init() {
    REGISTRY.call_once(|| Registry::new().expect("cannot create the device model roots"));
}

pub(crate) struct Registry {
    devices: Arc<Dir>,
    virtual_dir: Arc<Dir>,
    bus: Arc<Dir>,
    class: Arc<Dir>,
    dev_char: Arc<Dir>,
    dev_block: Arc<Dir>,
    virtual_glue_dirs: GlueDirs,
    subsystems: Mutex<Vec<Arc<dyn SubsystemOps>>>,
}

impl Registry {
    pub(crate) fn devices_root(&self) -> &Arc<Dir> {
        &self.devices
    }

    pub(crate) fn bus_root(&self) -> &Arc<Dir> {
        &self.bus
    }

    pub(crate) fn class_root(&self) -> &Arc<Dir> {
        &self.class
    }

    pub(crate) fn dev_index(&self, kind: DevKind) -> &Arc<Dir> {
        match kind {
            DevKind::Char => &self.dev_char,
            DevKind::Block => &self.dev_block,
        }
    }

    pub(crate) fn keep_subsystem(&self, subsystem: Arc<dyn SubsystemOps>) {
        self.subsystems.lock().push(subsystem);
    }

    pub(crate) fn attach_into_virtual_glue_dir(
        &self,
        class: &str,
        child: Arc<dyn SysObj>,
    ) -> Result<Arc<Dir>> {
        self.virtual_glue_dirs
            .attach_into(class, self.virtual_dir.as_ref(), child)
    }

    pub(crate) fn drop_virtual_glue_dir_if_empty(&self, class: &str) {
        self.virtual_glue_dirs
            .drop_if_empty(class, self.virtual_dir.as_ref());
    }

    fn new() -> Result<Self> {
        let devices = Dir::new(SysStr::from("devices"));
        let virtual_dir = Dir::new(SysStr::from("virtual"));
        let bus = Dir::new(SysStr::from("bus"));
        let class = Dir::new(SysStr::from("class"));
        let dev = Dir::new(SysStr::from("dev"));
        let dev_char = Dir::new(SysStr::from("char"));
        let dev_block = Dir::new(SysStr::from("block"));

        devices.attach_child(virtual_dir.clone())?;
        dev.attach_child(dev_char.clone())?;
        dev.attach_child(dev_block.clone())?;
        let root = aster_systree::primary_tree().root();
        root.add_child(devices.clone())?;
        root.add_child(bus.clone())?;
        root.add_child(class.clone())?;
        root.add_child(dev)?;

        Ok(Self {
            devices,
            virtual_dir,
            bus,
            class,
            dev_char,
            dev_block,
            virtual_glue_dirs: GlueDirs::new(),
            subsystems: Mutex::new(Vec::new()),
        })
    }
}

static REGISTRY: Once<Registry> = Once::new();
