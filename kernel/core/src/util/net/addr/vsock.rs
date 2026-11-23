use super::family::CSocketAddrFamily;
use crate::{net::socket::vsock::VsockSocketAddr, prelude::*};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct CSocketAddrVm {
    svm_family: u16,
    svm_reserved1: u16,
    svm_port: u32,
    svm_cid: u32,
    svm_zero: [u8; 4],
}

impl From<VsockSocketAddr> for CSocketAddrVm {
    fn from(value: VsockSocketAddr) -> Self {
        Self {
            svm_family: CSocketAddrFamily::AF_VSOCK as u16,
            svm_reserved1: 0,
            svm_port: value.port,
            svm_cid: value.cid,
            svm_zero: [0; 4],
        }
    }
}

impl From<CSocketAddrVm> for VsockSocketAddr {
    fn from(value: CSocketAddrVm) -> Self {
        debug_assert_eq!(value.svm_family, CSocketAddrFamily::AF_VSOCK as u16);
        Self {
            cid: value.svm_cid,
            port: value.svm_port,
        }
    }
}
