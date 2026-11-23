use core::{any::Any, fmt::Debug};

use smoltcp::{
    iface::{Config, Interface},
    phy::Device,
};
pub use smoltcp::{
    phy::{Checksum, ChecksumCapabilities, DeviceCapabilities, Medium},
    wire::EthernetAddress,
};

use crate::{
    packet::{FreshTxPacket, LinkLayer, RxPacket, TxPacket},
    time::Instant,
};

mod loopback;
pub use loopback::Loopback;

#[derive(Clone, Copy, Debug)]
pub enum NetError {
    NotReady,
    Busy,
    NoMemory,
}

pub trait AnyNetworkDevice: Send + Sync + Any + Debug {
    fn mac_addr(&self) -> EthernetAddress;
    fn capabilities(&self) -> DeviceCapabilities;

    fn can_receive(&self) -> bool;
    fn can_send(&self) -> bool;

    fn receive(&mut self) -> Result<RxPacket<LinkLayer>, NetError>;

    fn send(&mut self, packet: TxPacket<LinkLayer>) -> Result<(), NetError>;

    fn alloc_tx_buffer(payload_len: usize) -> Result<FreshTxPacket, ostd::Error>
    where
        Self: Sized;

    fn free_processed_tx_buffers(&mut self);

    fn notify_poll_end(&mut self);
}

pub trait WithDevice: Send + Sync {
    type Device: AnyNetworkDevice;

    fn with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut dyn AnyNetworkDevice) -> R;
}

pub(crate) fn new_interface(
    config: Config,
    capabilities: DeviceCapabilities,
    now: Instant,
) -> Interface {
    let mut device = CapabilitiesDevice(capabilities);
    Interface::new(config, &mut device, now)
}

struct CapabilitiesDevice(DeviceCapabilities);

impl Device for CapabilitiesDevice {
    type RxToken<'a> = <smoltcp::phy::Loopback as Device>::RxToken<'a>;
    type TxToken<'a> = <smoltcp::phy::Loopback as Device>::TxToken<'a>;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        None
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        None
    }

    fn capabilities(&self) -> DeviceCapabilities {
        self.0.clone()
    }
}
