#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

macro_rules! __log_prefix {
    () => {
        "device: "
    };
}

pub mod bus;
pub mod class;
pub mod common;
pub mod hooks;
#[cfg(ktest)]
mod test;
pub mod uevent;

use component::{ComponentInitError, init_component};

pub use self::common::registration::{
    add as add_device, register_legacy_virtual_node, remove as remove_device,
};
use self::common::registry;

#[cfg(ktest)]
pub fn init_for_ktest() {
    aster_systree::init_for_ktest();
    registry::init();
}

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    registry::init();
    Ok(())
}
