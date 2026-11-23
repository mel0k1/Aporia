#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use component::{ComponentInitError, init_component};

macro_rules! __log_prefix {
    () => {
        "uart: "
    };
}

#[cfg_attr(target_arch = "x86_64", path = "arch/x86/mod.rs")]
#[cfg_attr(target_arch = "riscv64", path = "arch/riscv/mod.rs")]
#[cfg_attr(target_arch = "loongarch64", path = "arch/loongarch/mod.rs")]
#[cfg_attr(target_arch = "aarch64", path = "arch/arm/mod.rs")]
mod arch;

mod console;

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    arch::init();
    Ok(())
}
