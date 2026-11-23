#![no_std]
#![deny(unsafe_code)]
#![expect(rustdoc::private_intra_doc_links)]

#[macro_use]
extern crate ostd_pod;

macro_rules! __log_prefix {
    () => {
        "bigtcp: "
    };
}

pub mod boolean_value;
pub mod device;
pub mod errors;
pub mod ext;
pub mod iface;
pub mod packet;
pub mod socket;
pub mod socket_table;
pub mod time;
pub mod wire;

extern crate alloc;
