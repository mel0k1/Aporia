#![no_std]
#![deny(unsafe_code)]

use aster_util::ranged_integer::{RangedU16, RangedU32};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct DeviceId(u32);

impl DeviceId {
    pub const fn new(major: MajorId, minor: MinorId) -> Self {
        Self(((major.get() as u32) << 20) | minor.get())
    }

    pub const fn null() -> Self {
        Self(0)
    }

    pub fn to_raw(&self) -> u32 {
        self.0
    }

    pub fn major(&self) -> MajorId {
        MajorId::new((self.0 >> 20) as u16)
    }

    pub fn minor(&self) -> MinorId {
        MinorId::new(self.0 & 0xf_ffff)
    }

    pub fn is_null(self) -> bool {
        self.0 == 0
    }

    pub fn is_anonymous(self) -> bool {
        self.major().get() == 0 && self.minor().get() >= 1
    }
}

impl DeviceId {
    pub fn from_encoded_u64(raw: u64) -> Option<Self> {
        let (major, minor) = decode_device_numbers(raw);
        let major = MajorId::try_from(major as u16).ok()?;
        let minor = MinorId::try_from(minor).ok()?;
        Some(Self::new(major, minor))
    }

    pub fn as_encoded_u64(&self) -> u64 {
        encode_device_numbers(self.major().get() as u32, self.minor().get())
    }
}

pub fn decode_device_numbers(raw: u64) -> (u32, u32) {
    let major = ((raw >> 32) & 0xffff_f000 | (raw >> 8) & 0x0000_0fff) as u32;
    let minor = ((raw >> 12) & 0xffff_ff00 | raw & 0x0000_00ff) as u32;
    (major, minor)
}

pub fn encode_device_numbers(major: u32, minor: u32) -> u64 {
    let major = major as u64;
    let minor = minor as u64;
    ((major & 0xffff_f000) << 32)
        | ((major & 0x0000_0fff) << 8)
        | ((minor & 0xffff_ff00) << 12)
        | (minor & 0x0000_00ff)
}

const MAX_MAJOR_ID: u16 = 0x0fff;

const MAX_MINOR_ID: u32 = 0x000f_ffff;

pub type MajorId = RangedU16<0, MAX_MAJOR_ID>;

pub type MinorId = RangedU32<0, MAX_MINOR_ID>;
