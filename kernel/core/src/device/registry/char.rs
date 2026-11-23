use core::ops::Range;

use device_id::{DeviceId, MajorId};

use crate::{
    device::Device,
    fs::devtmpfs::{self, DevtmpfsNode},
    prelude::*,
};

static DEVICE_REGISTRY: Mutex<BTreeMap<u32, Arc<dyn Device>>> = Mutex::new(BTreeMap::new());

pub fn register(device: Arc<dyn Device>) -> Result<()> {
    let mut registry = DEVICE_REGISTRY.lock();
    let id = device.id().to_raw();
    if registry.contains_key(&id) {
        return Err(errno!(EEXIST, "the char device already exists"));
    }
    registry.insert(id, device.clone());

    if let Some(meta) = device.devtmpfs_meta()
        && let Err(error) =
            devtmpfs::create_node(DevtmpfsNode::new(device.type_(), device.id(), meta))
    {
        registry.remove(&id);
        return Err(error);
    }
    Ok(())
}

pub fn unregister(id: DeviceId) -> Result<Arc<dyn Device>> {
    let mut registry = DEVICE_REGISTRY.lock();
    let device = registry
        .remove(&id.to_raw())
        .ok_or_else(|| errno!(ENOENT, "the char device does not exist"))?;

    if let Some(meta) = device.devtmpfs_meta()
        && let Err(error) =
            devtmpfs::delete_node(DevtmpfsNode::new(device.type_(), device.id(), meta))
    {
        warn!(
            "failed to delete devtmpfs node for char device {:?}: {:?}",
            id, error
        );
    }
    Ok(device)
}

pub(super) fn lookup(id: DeviceId) -> Option<Arc<dyn Device>> {
    DEVICE_REGISTRY.lock().get(&id.to_raw()).cloned()
}

pub(crate) const MAX_MAJOR: u16 = 511;

const DYNAMIC_MAJOR_ID_RANGES: [Range<u16>; 2] = [234..255, 384..512];

static MAJORS: Mutex<BTreeSet<u16>> = Mutex::new(BTreeSet::new());

pub(crate) fn acquire_major(major: MajorId) -> Result<MajorIdOwner> {
    if major.get() > MAX_MAJOR {
        return Err(errno!(EINVAL, "the major ID is invalid"));
    }

    if MAJORS.lock().insert(major.get()) {
        Ok(MajorIdOwner(major))
    } else {
        Err(errno!(EEXIST, "the major ID has already been acquired"))
    }
}

#[expect(dead_code)]
pub(crate) fn allocate_major() -> Result<MajorIdOwner> {
    let mut majors = MAJORS.lock();

    for id in DYNAMIC_MAJOR_ID_RANGES
        .iter()
        .flat_map(|range| range.clone().rev())
    {
        if majors.insert(id) {
            return Ok(MajorIdOwner(MajorId::new(id)));
        }
    }

    Err(errno!(ENOSPC, "no more major IDs are available"))
}

pub(crate) struct MajorIdOwner(MajorId);

impl MajorIdOwner {
    pub(crate) fn get(&self) -> MajorId {
        self.0
    }
}

impl Drop for MajorIdOwner {
    fn drop(&mut self) {
        MAJORS.lock().remove(&self.0.get());
    }
}
