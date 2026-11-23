use alloc::{collections::btree_map::BTreeMap, string::String, sync::Arc};

use ostd::sync::SpinLock;
use spin::Once;

use crate::device::entropy::device::EntropyDevice;

pub mod device;

fn register_device(name: String, device: Arc<EntropyDevice>) {
    let mut entropy_devs = ENTROPY_DEVICE_TABLE.get().unwrap().lock();

    entropy_devs.insert(name, device);
}

pub fn first_device() -> Option<Arc<EntropyDevice>> {
    let entropy_devs = ENTROPY_DEVICE_TABLE.get().unwrap().lock();

    entropy_devs.values().next().cloned()
}

pub(crate) fn init() {
    ENTROPY_DEVICE_TABLE.call_once(|| SpinLock::new(BTreeMap::new()));
}

static ENTROPY_DEVICE_TABLE: Once<SpinLock<BTreeMap<String, Arc<EntropyDevice>>>> = Once::new();
