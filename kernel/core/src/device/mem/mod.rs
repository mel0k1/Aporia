mod file;

use aster_device::{
    class::{self, Class, ClassDevice, ClassHandle},
    common::{AnyDevice, Attr, DevNode, DevNum},
};
use device_id::{DeviceId, MajorId, MinorId};
use file::MemFile;
pub(crate) use file::{getrandom, geturandom};
use spin::Once;

use super::{
    Device, DeviceType,
    registry::char::{self, MajorIdOwner},
};
use crate::{
    fs::{
        devtmpfs::DevtmpfsNodeMeta,
        file::{PerOpenFileOps, mkmod},
    },
    prelude::*,
};

pub(super) fn init_in_first_kthread() {
    MEM_MAJOR.call_once(|| char::acquire_major(MajorId::new(1)).unwrap());
    MEM_CLASS.call_once(|| class::register(MemClass).unwrap());

    add_device(MemFile::Full).unwrap();
    add_device(MemFile::Null).unwrap();
    add_device(MemFile::Random).unwrap();
    add_device(MemFile::Urandom).unwrap();
    add_device(MemFile::Zero).unwrap();
}

pub(super) struct MemClass;

impl Class for MemClass {
    const NAME: &'static str = "mem";
    type Device = MemFile;

    fn devnode(&self, dev: &ClassDevice<Self>) -> Option<DevNode> {
        let mode = match dev.payload() {
            MemFile::Full | MemFile::Null | MemFile::Random | MemFile::Urandom | MemFile::Zero => {
                mkmod!(a+rw)
            }
            MemFile::Kmsg => mkmod!(a+r, u+w),
            _ => return None,
        };
        Some(DevNode {
            path: None,
            mode: Some(mode.bits()),
        })
    }

    fn dev_attrs(&self) -> &'static [Attr<ClassDevice<Self>>] {
        &[]
    }

    fn append_uevent_vars(
        &self,
        _dev: &ClassDevice<Self>,
        _vars: &mut aster_device::uevent::UeventVars,
    ) -> aster_device::common::Result<()> {
        Ok(())
    }
}

pub(super) type MemDevice = ClassDevice<MemClass>;

impl Device for MemDevice {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        self.devnum()
            .expect("memory devices always have a device number")
            .id()
    }

    fn devtmpfs_meta(&self) -> Option<DevtmpfsNodeMeta> {
        None
    }

    fn open(&self) -> Result<Box<dyn PerOpenFileOps>> {
        Ok(Box::new(*self.payload()))
    }
}

static MEM_MAJOR: Once<MajorIdOwner> = Once::new();
static MEM_CLASS: Once<Arc<ClassHandle<MemClass>>> = Once::new();

fn add_device(file: MemFile) -> Result<()> {
    let class = MEM_CLASS.get().unwrap();
    let id = DeviceId::new(MEM_MAJOR.get().unwrap().get(), MinorId::new(file.minor()));
    let device = MemDevice::builder(class, file.name(), file)
        .devnum(DevNum::char(id))
        .build();
    aster_device::add_device(&device)?;
    if let Err(error) = char::register(device.clone()) {
        let _ = aster_device::remove_device(&device);
        return Err(error);
    }
    Ok(())
}
