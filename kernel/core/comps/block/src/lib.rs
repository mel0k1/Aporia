#![no_std]
#![deny(unsafe_code)]
#![feature(step_trait)]

extern crate alloc;
#[macro_use]
extern crate ostd_pod;

macro_rules! __log_prefix {
    () => {
        "block: "
    };
}

pub mod bio;
mod device_id;
pub mod id;
mod impl_block_device;
mod partition;
mod prelude;
pub mod request_queue;

use ::device_id::DeviceId;
use component::{ComponentInitError, init_component};
use ostd::sync::Mutex;

use self::{
    bio::{BioEnqueueError, SubmittedBio},
    prelude::*,
};
pub use self::{
    device_id::{MAX_MAJOR, MajorIdOwner, acquire_major, allocate_major},
    partition::PartitionManager,
};

pub const BLOCK_SIZE: usize = ostd::mm::PAGE_SIZE;
pub const SECTOR_SIZE: usize = 512;

pub const DEVICE_MINORS: u32 = 16;

pub trait BlockDevice: Send + Sync + Any + Debug {
    fn enqueue(&self, bio: SubmittedBio) -> Result<(), BioEnqueueError>;

    fn metadata(&self) -> BlockDeviceMeta;

    fn name(&self) -> &str;

    fn id(&self) -> DeviceId;

    fn is_partition(&self) -> bool {
        false
    }

    fn partition_manager(&self) -> Option<&PartitionManager> {
        None
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BlockDeviceMeta {
    pub max_nr_segments_per_bio: usize,
    pub nr_sectors: usize,
}

impl dyn BlockDevice {
    pub fn downcast_ref<T: BlockDevice>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref::<T>()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Registered,
    NotFound,
    InvalidArgs,
    IdAcquired,
    IdExhausted,
}

pub fn register(device: Arc<dyn BlockDevice>) -> Result<(), Error> {
    let mut registry = DEVICE_REGISTRY.lock();
    let id = device.id().to_raw();
    if registry.contains_key(&id) {
        return Err(Error::Registered);
    }
    registry.insert(id, device);

    Ok(())
}

pub fn unregister(id: DeviceId) -> Result<Arc<dyn BlockDevice>, Error> {
    DEVICE_REGISTRY
        .lock()
        .remove(&id.to_raw())
        .ok_or(Error::NotFound)
}

pub fn collect_all() -> Vec<Arc<dyn BlockDevice>> {
    DEVICE_REGISTRY.lock().values().cloned().collect()
}

pub fn lookup(id: DeviceId) -> Option<Arc<dyn BlockDevice>> {
    DEVICE_REGISTRY.lock().get(&id.to_raw()).cloned()
}

pub fn lookup_by_name(name: &str) -> Option<Arc<dyn BlockDevice>> {
    DEVICE_REGISTRY
        .lock()
        .values()
        .find(|device| device.name() == name)
        .cloned()
}

pub fn scan_partitions() {
    let devices = collect_all();
    for device in devices {
        if device.is_partition() {
            continue;
        }

        let Some(partition_manager) = device.partition_manager() else {
            continue;
        };

        let Some(partition_infos) = partition::parse(&device) else {
            continue;
        };

        partition_manager.update(&device, partition_infos);
    }
}

static DEVICE_REGISTRY: Mutex<BTreeMap<u32, Arc<dyn BlockDevice>>> = Mutex::new(BTreeMap::new());

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    device_id::init();

    Ok(())
}
