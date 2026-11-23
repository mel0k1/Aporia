mod conn_id;
mod connection;
mod listener;
mod port;
mod space;
mod timer;

use core::time::Duration;

pub(super) use connection::{Connection, connect::ConnectResult};
pub(super) use listener::Listener;
pub(super) use port::BoundPort;

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const DEFAULT_CLOSE_TIMEOUT: Duration = Duration::from_secs(8);
const DEFAULT_RX_BUF_SIZE: usize = 256 * 1024;
const DEFAULT_TX_BUF_SIZE: usize = 256 * 1024;
const MAX_BACKLOG: usize = 4096;

const CREDIT_UPDATE_THRESHOLD: u32 = (DEFAULT_RX_BUF_SIZE / 4) as u32;

fn process_rx_callback() {
    if let Ok(vsock_space) = space::vsock_space() {
        vsock_space.process_rx();
    }
}

fn process_event_callback() {
    if let Ok(vsock_space) = space::vsock_space() {
        vsock_space.process_transport_event();
    }
}

pub(super) fn init() {
    use aster_virtio::device::socket::DEVICE_NAME;

    let Some(device) = aster_virtio::device::socket::get_device(DEVICE_NAME) else {
        return;
    };

    device.init_rx_callback(process_rx_callback);
    device.init_event_callback(process_event_callback);
    space::init(device);

    timer::init();
}
