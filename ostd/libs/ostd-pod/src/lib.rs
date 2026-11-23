#![doc = include_str!("../README.md")]
#![no_std]
#![deny(unsafe_code)]

pub use zerocopy::{FromBytes, FromZeros, Immutable, IntoBytes, KnownLayout};

pub mod array_helper;

pub trait Pod: FromBytes + IntoBytes + KnownLayout + Immutable + Copy {
    #[track_caller]
    fn from_bytes(bytes: &[u8]) -> Self {
        <Self as FromBytes>::read_from_bytes(bytes).unwrap()
    }

    #[track_caller]
    fn from_first_bytes(bytes: &[u8]) -> Self {
        <Self as FromBytes>::read_from_prefix(bytes).unwrap().0
    }
}

impl<T: FromBytes + IntoBytes + KnownLayout + Immutable + Copy> Pod for T {}

#[cfg(feature = "macros")]
pub use ostd_pod_macros::{derive, pod_union};
#[cfg(feature = "macros")]
pub use padding_struct::padding_struct;
