use alloc::{boxed::Box, sync::Arc};
use core::{
    fmt::Debug,
    ops::{Deref, DerefMut},
};

use aster_pci::cfg_space::BarAccess;
use aster_util::safe_ptr::SafePtr;
use ostd::{
    arch::device::io_port::{PortRead, PortWrite},
    io::IoMem,
    irq::IrqCallbackFunction,
    mm::{PodOnce, dma::DmaCoherent},
};
use ostd_pod::Pod;

use self::{mmio::virtio_mmio_init, pci::virtio_pci_init};
use crate::{
    VirtioDeviceType,
    queue::{AvailRing, Descriptor, UsedRing},
};

pub(crate) mod mmio;
pub(crate) mod pci;

pub(crate) trait VirtioTransport: Sync + Send + Debug {
    fn device_type(&self) -> VirtioDeviceType;

    fn read_device_features(&self) -> u64;

    fn write_driver_features(&mut self, features: u64) -> Result<(), VirtioTransportError>;

    fn read_device_status(&self) -> DeviceStatus;

    fn write_device_status(&mut self, status: DeviceStatus) -> Result<(), VirtioTransportError>;

    fn device_config_mem(&self) -> Option<IoMem>;

    fn device_config_bar(&self) -> Option<(BarAccess, usize)>;

    fn num_queues(&self) -> u16;

    fn set_queue(
        &mut self,
        idx: u16,
        queue_size: u16,
        descriptor_ptr: &SafePtr<Descriptor, Arc<DmaCoherent>>,
        avail_ring_ptr: &SafePtr<AvailRing, Arc<DmaCoherent>>,
        used_ring_ptr: &SafePtr<UsedRing, Arc<DmaCoherent>>,
    ) -> Result<(), VirtioTransportError>;

    fn max_queue_size(&self, idx: u16) -> Result<u16, VirtioTransportError>;

    fn notify_config(&self, idx: usize) -> ConfigManager<u32>;

    fn is_legacy_version(&self) -> bool;

    fn register_queue_callback(
        &mut self,
        index: u16,
        func: Box<IrqCallbackFunction>,
        single_interrupt: bool,
    ) -> Result<(), VirtioTransportError>;

    fn register_cfg_callback(
        &mut self,
        func: Box<IrqCallbackFunction>,
    ) -> Result<(), VirtioTransportError>;
}

#[derive(Debug)]
pub(crate) struct ConfigManager<T: Pod> {
    modern_space: Option<SafePtr<T, IoMem>>,
    legacy_space: Option<(BarAccess, usize)>,
}

impl<T: Pod> ConfigManager<T> {
    pub(super) fn new(
        modern_space: Option<SafePtr<T, IoMem>>,
        legacy_space: Option<(BarAccess, usize)>,
    ) -> Self {
        Self {
            modern_space,
            legacy_space,
        }
    }

    pub(super) fn is_modern(&self) -> bool {
        self.modern_space.is_some()
    }

    fn read_modern<V: PodOnce + PortRead>(&self, offset: usize) -> Result<V, VirtioTransportError> {
        let Some(safe_ptr) = self.modern_space.as_ref() else {
            return Err(VirtioTransportError::InvalidArgs);
        };

        let field_ptr: SafePtr<V, &IoMem> = {
            let mut ptr = safe_ptr.borrow_vm();
            ptr.byte_add(offset);
            ptr.cast()
        };

        field_ptr
            .read_once()
            .map_err(|_| VirtioTransportError::DeviceStatusError)
    }

    fn read_legacy<V: PodOnce + PortRead>(&self, offset: usize) -> Result<V, VirtioTransportError> {
        let Some((bar, base)) = self.legacy_space.as_ref() else {
            return Err(VirtioTransportError::InvalidArgs);
        };

        bar.read_once(base + offset)
            .map_err(|_| VirtioTransportError::DeviceStatusError)
    }

    pub(super) fn read_once<V: PodOnce + PortRead>(
        &self,
        offset: usize,
    ) -> Result<V, VirtioTransportError> {
        debug_assert!(offset + size_of::<V>() <= size_of::<T>());
        if self.is_modern() {
            self.read_modern(offset)
        } else {
            self.read_legacy(offset)
        }
    }

    fn write_modern<V: PodOnce + PortWrite>(
        &self,
        offset: usize,
        value: V,
    ) -> Result<(), VirtioTransportError> {
        let Some(safe_ptr) = self.modern_space.as_ref() else {
            return Err(VirtioTransportError::InvalidArgs);
        };

        let field_ptr: SafePtr<V, &IoMem> = {
            let mut ptr = safe_ptr.borrow_vm();
            ptr.byte_add(offset);
            ptr.cast()
        };

        field_ptr
            .write_once(&value)
            .map_err(|_| VirtioTransportError::DeviceStatusError)
    }

    fn write_legacy<V: PodOnce + PortWrite>(
        &self,
        offset: usize,
        value: V,
    ) -> Result<(), VirtioTransportError> {
        let Some((bar, base)) = self.legacy_space.as_ref() else {
            return Err(VirtioTransportError::InvalidArgs);
        };

        bar.write_once(base + offset, value)
            .map_err(|_| VirtioTransportError::DeviceStatusError)
    }

    pub(super) fn write_once<V: PodOnce + PortWrite>(
        &self,
        offset: usize,
        value: V,
    ) -> Result<(), VirtioTransportError> {
        debug_assert!(offset + size_of::<V>() <= size_of::<T>());
        if self.is_modern() {
            self.write_modern(offset, value)
        } else {
            self.write_legacy(offset, value)
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum VirtioTransportError {
    DeviceStatusError,
    InvalidArgs,
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub(crate) struct DeviceStatus: u8 {
        const ACKNOWLEDGE        = 1;

        const DRIVER             = 2;

        const FAILED             = 128;

        const FEATURES_OK        = 8;

        const DRIVER_OK          = 4;

        const DEVICE_NEEDS_RESET = 64;
    }
}

#[derive(Debug)]
pub(crate) struct DeviceTransport {
    inner: Box<dyn VirtioTransport>,
    is_completed: bool,
}

impl DeviceTransport {
    pub(super) fn new(transport: Box<dyn VirtioTransport>) -> Self {
        Self {
            inner: transport,
            is_completed: false,
        }
    }

    pub(crate) fn finish_init(&mut self) {
        self.is_completed = true;
        let status = self.inner.read_device_status() | DeviceStatus::DRIVER_OK;
        self.inner.write_device_status(status).unwrap();
    }
}

impl Deref for DeviceTransport {
    type Target = Box<dyn VirtioTransport>;

    fn deref(&self) -> &Box<dyn VirtioTransport> {
        &self.inner
    }
}

impl DerefMut for DeviceTransport {
    fn deref_mut(&mut self) -> &mut Box<dyn VirtioTransport> {
        &mut self.inner
    }
}

impl Drop for DeviceTransport {
    fn drop(&mut self) {
        if !self.is_completed {
            let status = self.inner.read_device_status();
            let _ = self
                .inner
                .write_device_status(status | DeviceStatus::FAILED);
        }
    }
}

pub(super) fn init() {
    virtio_pci_init();
    virtio_mmio_init();
}
