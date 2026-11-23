#![short_vis_path::add(mmio)]

use alloc::{collections::VecDeque, fmt::Debug, sync::Arc, vec::Vec};

use ostd::{debug, error};

use super::common_device::MmioCommonDevice;

pub(in mmio) trait MmioDevice: Sync + Send + Debug {
    fn device_id(&self) -> u32;
}

pub(in mmio) trait MmioDriver: Sync + Send + Debug {
    fn probe(
        &self,
        device: MmioCommonDevice,
    ) -> Result<Arc<dyn MmioDevice>, (aster_device::common::Error, MmioCommonDevice)>;
}

pub(in mmio) struct MmioBus {
    common_devices: VecDeque<MmioCommonDevice>,
    devices: Vec<Arc<dyn MmioDevice>>,
    drivers: Vec<Arc<dyn MmioDriver>>,
}

impl MmioBus {
    pub(in mmio) fn register_driver(&mut self, driver: Arc<dyn MmioDriver>) {
        debug!("Register driver: {:#x?}", driver);
        let length = self.common_devices.len();
        for _ in (0..length).rev() {
            let common_device = self.common_devices.pop_front().unwrap();
            let device_id = common_device.read_device_id().unwrap();
            let device = match driver.probe(common_device) {
                Ok(device) => {
                    debug_assert!(device_id == device.device_id());
                    self.devices.push(device);
                    continue;
                }
                Err((err, device)) => {
                    if !matches!(err, aster_device::common::Error::ProbeFailed) {
                        error!("MMIO device construction failed, reason: {:?}", err);
                    }
                    device
                }
            };
            self.common_devices.push_back(device);
        }
        self.drivers.push(driver);
    }

    pub(super) fn register_mmio_device(&mut self, mut mmio_device: MmioCommonDevice) {
        let device_id = mmio_device.read_device_id().unwrap();
        for driver in self.drivers.iter() {
            mmio_device = match driver.probe(mmio_device) {
                Ok(device) => {
                    debug_assert!(device_id == device.device_id());
                    self.devices.push(device);
                    return;
                }
                Err((err, common_device)) => {
                    if !matches!(err, aster_device::common::Error::ProbeFailed) {
                        error!("MMIO device construction failed, reason: {:?}", err);
                    }
                    common_device
                }
            };
        }
        self.common_devices.push_back(mmio_device);
    }

    pub(super) const fn new() -> Self {
        Self {
            common_devices: VecDeque::new(),
            devices: Vec::new(),
            drivers: Vec::new(),
        }
    }
}
