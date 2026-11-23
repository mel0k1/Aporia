#![doc = include_str!("../README.md")]
#![cfg_attr(not(test), no_std)]

#[derive(Clone, Copy, Debug)]
pub struct TryFromIntError;

#[cfg(feature = "derive")]
pub use int_to_c_enum_derive::TryFromInt;
