#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate aster_drm as _;
#[cfg(target_arch = "x86_64")]
extern crate aster_i8042 as _;
extern crate aster_simpledrm as _;
extern crate aster_uart as _;

#[ostd::main]
fn main() {
    aster_core::boot();
}
