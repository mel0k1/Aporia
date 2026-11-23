#![no_std]
#![deny(unsafe_code)]
#![feature(custom_inner_attributes)]

extern crate alloc;
#[macro_use]
extern crate ostd_pod;

use alloc::boxed::Box;
use core::hint::spin_loop;

use aster_block::MajorIdOwner;
use bitflags::bitflags;
use component::{ComponentInitError, init_component};
use device::{
    VirtioDeviceType, block::device::BlockDevice, console::device::ConsoleDevice,
    entropy::device::EntropyDevice, filesystem::device::FileSystemDevice,
    input::device::InputDevice, network::device::NetworkDevice, socket::device::SocketDevice,
};
use ostd::{error, warn};
use spin::Once;
use transport::{DeviceStatus, mmio::VIRTIO_MMIO_DRIVER};

use crate::transport::{DeviceTransport, VirtioTransport};

macro_rules! __log_prefix {
    () => {
        "virtio: "
    };
}

pub mod device;
mod dma_buf;
mod id_alloc;
mod queue;
mod transport;

static VIRTIO_BLOCK_MAJOR_ID: Once<MajorIdOwner> = Once::new();

#[init_component]
fn virtio_component_init() -> Result<(), ComponentInitError> {
    VIRTIO_BLOCK_MAJOR_ID.call_once(|| aster_block::allocate_major().unwrap());

    transport::init();

    device::entropy::init();
    device::network::init();
    device::socket::init();

    while let Some(mut transport) = pop_device_transport() {
        let device_type = transport.device_type();

        transport
            .write_device_status(DeviceStatus::empty())
            .unwrap();
        while transport.read_device_status() != DeviceStatus::empty() {
            spin_loop();
        }

        transport
            .write_device_status(DeviceStatus::ACKNOWLEDGE)
            .unwrap();

        transport
            .write_device_status(DeviceStatus::ACKNOWLEDGE | DeviceStatus::DRIVER)
            .unwrap();

        negotiate_features(&mut transport);

        if !transport.is_legacy_version() {
            let status =
                DeviceStatus::ACKNOWLEDGE | DeviceStatus::DRIVER | DeviceStatus::FEATURES_OK;
            transport.write_device_status(status).unwrap();

            let status = transport.read_device_status();
            if !status.contains(DeviceStatus::FEATURES_OK) {
                error!(
                    "Device rejected negotiated features, device type: {:?}",
                    device_type
                );
                transport
                    .write_device_status(status | DeviceStatus::FAILED)
                    .unwrap();
                continue;
            }
        }

        let device_transport = DeviceTransport::new(transport);

        let res = match device_type {
            VirtioDeviceType::Block => BlockDevice::init(device_transport),
            VirtioDeviceType::Console => ConsoleDevice::init(device_transport),
            VirtioDeviceType::Entropy => EntropyDevice::init(device_transport),
            VirtioDeviceType::Input => InputDevice::init(device_transport),
            VirtioDeviceType::Network => NetworkDevice::init(device_transport),
            VirtioDeviceType::Socket => SocketDevice::init(device_transport),
            VirtioDeviceType::FileSystem => FileSystemDevice::init(device_transport),
            _ => {
                warn!("Found unimplemented device: {:?}", device_type);
                Ok(())
            }
        };
        if res.is_err() {
            error!(
                "Device initialization error: {:?}, device type: {:?}",
                res, device_type
            );
        }
    }
    Ok(())
}

fn pop_device_transport() -> Option<Box<dyn VirtioTransport>> {
    if let Some(device) = transport::pci::pop_device_transport() {
        return Some(device);
    }
    if let Some(device) = VIRTIO_MMIO_DRIVER.get().unwrap().pop_device_transport() {
        return Some(Box::new(device));
    }
    None
}

fn negotiate_features(transport: &mut Box<dyn VirtioTransport>) {
    let features = transport.read_device_features();
    let mask = ((1u64 << 24) - 1) | (((1u64 << 24) - 1) << 50);
    let device_specified_features = features & mask;
    let device_support_features = match transport.device_type() {
        VirtioDeviceType::Network => NetworkDevice::negotiate_features(device_specified_features),
        VirtioDeviceType::Block => BlockDevice::negotiate_features(device_specified_features),
        VirtioDeviceType::Input => InputDevice::negotiate_features(device_specified_features),
        VirtioDeviceType::Console => ConsoleDevice::negotiate_features(device_specified_features),
        VirtioDeviceType::Socket => SocketDevice::negotiate_features(device_specified_features),
        VirtioDeviceType::FileSystem => {
            FileSystemDevice::negotiate_features(device_specified_features)
        }
        _ => device_specified_features,
    };
    let mut support_feature = Feature::from_bits_truncate(features);
    support_feature.remove(Feature::RING_EVENT_IDX);
    transport
        .write_driver_features(features & (support_feature.bits() | device_support_features))
        .unwrap();
}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    struct Feature: u64 {

        const NOTIFY_ON_EMPTY    = 1 << 24;
        const ANY_LAYOUT         = 1 << 27;
        const RING_INDIRECT_DESC = 1 << 28;
        const RING_EVENT_IDX     = 1 << 29;
        const UNUSED             = 1 << 30;
        const VERSION_1          = 1 << 32;

        const ACCESS_PLATFORM    = 1 << 33;
        const RING_PACKED        = 1 << 34;
        const IN_ORDER           = 1 << 35;
        const ORDER_PLATFORM     = 1 << 36;
        const SR_IOV             = 1 << 37;
        const NOTIFICATION_DATA  = 1 << 38;
        const NOTIF_CONFIG_DATA  = 1 << 39;
        const RING_RESET         = 1 << 40;
    }
}
