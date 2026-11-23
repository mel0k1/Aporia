#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use component::{ComponentInitError, init_component};

macro_rules! __log_prefix {
    () => {
        "logger: "
    };
}

mod aster_logger;
mod console;

pub use console::_print;

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    aster_logger::init();
    Ok(())
}
