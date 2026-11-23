#![expect(dead_code)]

use alloc::{borrow::ToOwned, string::String, vec::Vec};
use core::fmt::Debug;

use ostd_pod::Pod;

#[derive(Clone, Debug)]
pub(in crate::arch) struct Drhd {
    header: DrhdHeader,
    device_scopes: Vec<DeviceScope>,
}

impl Drhd {
    pub(in crate::arch) fn register_base_addr(&self) -> u64 {
        self.header.register_base_addr
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct DrhdHeader {
    typ: u16,
    length: u16,
    flags: u8,
    size: u8,
    segment_num: u16,
    register_base_addr: u64,
}

#[derive(Clone, Debug)]
pub(in crate::arch) struct Rmrr {
    header: RmrrHeader,
    device_scopes: Vec<DeviceScope>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct RmrrHeader {
    typ: u16,
    length: u16,
    reserved: u16,
    segment_num: u16,
    reserved_memory_region_base_addr: u64,
    reserved_memory_region_limit_addr: u64,
}

#[derive(Clone, Debug)]
pub(in crate::arch) struct Atsr {
    header: AtsrHeader,
    device_scopes: Vec<DeviceScope>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct AtsrHeader {
    typ: u16,
    length: u16,
    flags: u8,
    reserved: u8,
    segment_num: u16,
}

#[padding_struct]
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(in crate::arch) struct Rhsa {
    typ: u16,
    length: u16,
    flags: u32,
    register_base_addr: u64,
    proximity_domain: u32,
}

#[derive(Clone, Debug)]
pub(in crate::arch) struct Andd {
    header: AnddHeader,
    acpi_object_name: String,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct AnddHeader {
    typ: u16,
    length: u16,
    reserved: [u8; 3],
    acpi_device_num: u8,
}

#[derive(Clone, Debug)]
pub(in crate::arch) struct Satc {
    header: SatcHeader,
    device_scopes: Vec<DeviceScope>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct SatcHeader {
    typ: u16,
    length: u16,
    flags: u8,
    reserved: u8,
    segment_num: u16,
}

#[derive(Clone, Debug)]
pub(in crate::arch) struct Sidp {
    header: SidpHeader,
    device_scopes: Vec<DeviceScope>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct SidpHeader {
    typ: u16,
    length: u16,
    reserved: u16,
    segment_num: u16,
}

#[derive(Clone, Debug)]
struct DeviceScope {
    header: DeviceScopeHeader,
    path: Vec<(u8, u8)>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct DeviceScopeHeader {
    typ: u8,
    length: u8,
    flags: u8,
    reserved: u8,
    enum_id: u8,
    start_bus_number: u8,
}

macro_rules! impl_from_bytes {
    ($(($struct:tt, $header_struct:tt),)*) => {
        $(impl $struct {
            #[doc = concat!("Parses a [`", stringify!($struct), "`] from bytes.")]
            #[doc = concat!(
                "This method may panic if the bytes do not represent a valid [`",
                stringify!($struct),
                "`].",
            )]
            pub(super) fn from_bytes(bytes: &[u8]) -> Self {
                let header = $header_struct::from_first_bytes(bytes);
                debug_assert_eq!(header.length as usize, bytes.len());

                let mut index = size_of::<$header_struct>();
                let mut device_scopes = Vec::new();
                while index != (header.length as usize) {
                    let val = DeviceScope::from_bytes_prefix(&bytes[index..]);
                    index += val.header.length as usize;
                    device_scopes.push(val);
                }

                Self{
                    header,
                    device_scopes,
                }
            }
        })*
    };
}

impl_from_bytes!(
    (Drhd, DrhdHeader),
    (Rmrr, RmrrHeader),
    (Atsr, AtsrHeader),
    (Satc, SatcHeader),
    (Sidp, SidpHeader),
);

impl DeviceScope {
    fn from_bytes_prefix(bytes: &[u8]) -> Self {
        let header = DeviceScopeHeader::from_first_bytes(bytes);
        debug_assert!((header.length as usize) <= bytes.len());

        let mut index = size_of::<DeviceScopeHeader>();
        debug_assert!((header.length as usize) >= index);

        let mut path = Vec::new();
        while index != (header.length as usize) {
            let val = (bytes[index], bytes[index + 1]);
            path.push(val);
            index += 2;
        }

        Self { header, path }
    }
}

impl Rhsa {
    pub(super) fn from_bytes(bytes: &[u8]) -> Self {
        let val = <Self as Pod>::from_first_bytes(bytes);
        debug_assert_eq!(val.length as usize, bytes.len());

        val
    }
}

impl Andd {
    pub(super) fn from_bytes(bytes: &[u8]) -> Self {
        let header = AnddHeader::from_first_bytes(bytes);
        debug_assert_eq!(header.length as usize, bytes.len());

        let header_len = size_of::<AnddHeader>();
        let acpi_object_name = core::str::from_utf8(&bytes[header_len..])
            .unwrap()
            .to_owned();

        Self {
            header,
            acpi_object_name,
        }
    }
}
