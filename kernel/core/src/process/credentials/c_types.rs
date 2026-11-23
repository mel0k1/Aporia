use crate::{prelude::*, process::Pid};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct CUserCapHeader {
    pub version: u32,
    pub pid: Pid,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct CUserCapData {
    pub effective: u32,
    pub permitted: u32,
    pub inheritable: u32,
}

pub(crate) const LINUX_CAPABILITY_VERSION_3: u32 = 0x20080522;
