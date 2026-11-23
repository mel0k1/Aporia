#![no_std]
#![deny(unsafe_code)]
#![feature(trait_alias)]

extern crate alloc;

use alloc::{collections::BTreeMap, string::String, sync::Arc, vec::Vec};

use aster_bigtcp::device::AnyNetworkDevice;
use aster_softirq::{
    BottomHalfDisabled, SoftIrqLine,
    softirq_id::{NETWORK_RX_SOFTIRQ_ID, NETWORK_TX_SOFTIRQ_ID},
};
use component::{ComponentInitError, init_component};
use ostd::sync::SpinLock;
use spin::Once;

pub trait NetDeviceCallback = Fn() + Send + Sync + 'static;

pub fn register_device(
    name: String,
    device: Arc<SpinLock<dyn AnyNetworkDevice, BottomHalfDisabled>>,
) {
    COMPONENT
        .get()
        .unwrap()
        .network_device_table
        .lock()
        .insert(name, NetworkDeviceIrqCallbackSet::new(device));
}

pub fn get_device(str: &str) -> Option<Arc<SpinLock<dyn AnyNetworkDevice, BottomHalfDisabled>>> {
    let table = COMPONENT.get().unwrap().network_device_table.lock();
    let callbacks = table.get(str)?;
    Some(callbacks.device.clone())
}

pub fn register_recv_callback(name: &str, callback: impl NetDeviceCallback) {
    let device_table = COMPONENT.get().unwrap().network_device_table.lock();
    let Some(callbacks) = device_table.get(name) else {
        return;
    };
    callbacks.recv_callbacks.lock().push(Arc::new(callback));
}

pub fn register_send_callback(name: &str, callback: impl NetDeviceCallback) {
    let device_table = COMPONENT.get().unwrap().network_device_table.lock();
    let Some(callbacks) = device_table.get(name) else {
        return;
    };
    callbacks.send_callbacks.lock().push(Arc::new(callback));
}

fn handle_rx_softirq() {
    let device_table = COMPONENT.get().unwrap().network_device_table.lock();
    for callback_set in device_table.values() {
        let recv_callbacks = callback_set.recv_callbacks.lock();
        for callback in recv_callbacks.iter() {
            callback();
        }
    }
}

fn handle_tx_softirq() {
    let device_table = COMPONENT.get().unwrap().network_device_table.lock();
    for callback_set in device_table.values() {
        let can_send = {
            let mut device = callback_set.device.lock();
            device.free_processed_tx_buffers();
            device.can_send()
        };

        if !can_send {
            continue;
        }

        let send_callbacks = callback_set.send_callbacks.lock();
        for callback in send_callbacks.iter() {
            callback();
        }
    }
}

pub fn raise_send_softirq() {
    SoftIrqLine::get(NETWORK_TX_SOFTIRQ_ID).raise();
}

pub fn raise_receive_softirq() {
    SoftIrqLine::get(NETWORK_RX_SOFTIRQ_ID).raise();
}

pub fn all_devices() -> Vec<(String, NetworkDeviceRef)> {
    let network_devs = COMPONENT.get().unwrap().network_device_table.lock();
    network_devs
        .iter()
        .map(|(name, callbacks)| (name.clone(), callbacks.device.clone()))
        .collect()
}

static COMPONENT: Once<Component> = Once::new();

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    let component = Component::init()?;
    COMPONENT.call_once(|| component);

    SoftIrqLine::get(NETWORK_TX_SOFTIRQ_ID).enable(handle_tx_softirq);
    SoftIrqLine::get(NETWORK_RX_SOFTIRQ_ID).enable(handle_rx_softirq);

    Ok(())
}

type NetDeviceCallbackListRef = Arc<SpinLock<Vec<Arc<dyn NetDeviceCallback>>, BottomHalfDisabled>>;
type NetworkDeviceRef = Arc<SpinLock<dyn AnyNetworkDevice, BottomHalfDisabled>>;

struct Component {
    network_device_table:
        SpinLock<BTreeMap<String, NetworkDeviceIrqCallbackSet>, BottomHalfDisabled>,
}

struct NetworkDeviceIrqCallbackSet {
    device: NetworkDeviceRef,
    recv_callbacks: NetDeviceCallbackListRef,
    send_callbacks: NetDeviceCallbackListRef,
}

impl NetworkDeviceIrqCallbackSet {
    fn new(device: NetworkDeviceRef) -> Self {
        Self {
            device,
            recv_callbacks: Arc::new(SpinLock::new(Vec::new())),
            send_callbacks: Arc::new(SpinLock::new(Vec::new())),
        }
    }
}

impl Component {
    fn init() -> Result<Self, ComponentInitError> {
        Ok(Self {
            network_device_table: SpinLock::new(BTreeMap::new()),
        })
    }
}
