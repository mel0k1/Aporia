use core::mem::offset_of;

use super::family::CSocketAddrFamily;
use crate::{net::socket::unix::UnixSocketAddr, prelude::*};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct CSocketAddrUnix {
    sun_family: u16,
    sun_path: [u8; Self::PATH_MAX_LEN],
}

impl CSocketAddrUnix {
    const PATH_MAX_LEN: usize = 108;

    const PATH_OFFSET: usize = offset_of!(Self, sun_path);

    const MIN_LEN: usize = Self::PATH_OFFSET;
    const MAX_LEN: usize = size_of::<Self>();
}

pub(super) fn into_c_bytes_and<R, F>(value: &UnixSocketAddr, f: F) -> R
where
    F: FnOnce(&[u8]) -> R,
{
    let mut bytes: [u8; CSocketAddrUnix::MAX_LEN + 1] = FromZeros::new_zeroed();

    bytes[..2].copy_from_slice(&(CSocketAddrFamily::AF_UNIX as u16).to_ne_bytes());
    const { assert!(CSocketAddrUnix::PATH_OFFSET == 2) };

    let sun_path = &mut bytes[CSocketAddrUnix::PATH_OFFSET..];

    let copied = match value {
        UnixSocketAddr::Unnamed => 0,
        UnixSocketAddr::Path(path) => {
            let bytes = path.to_bytes_with_nul();
            sun_path[..bytes.len()].copy_from_slice(bytes);
            bytes.len()
        }
        UnixSocketAddr::Abstract(name) => {
            let len = name.len();
            sun_path[0] = 0;
            sun_path[1..len + 1].copy_from_slice(&name[..]);
            len + 1
        }
    };

    f(&bytes[..CSocketAddrUnix::PATH_OFFSET + copied])
}

pub(super) fn from_c_bytes(bytes: &[u8]) -> Result<UnixSocketAddr> {
    if bytes.len() < CSocketAddrUnix::MIN_LEN {
        return Err(errno!(EINVAL, "the socket address length is too small"));
    }

    if bytes.len() > CSocketAddrUnix::MAX_LEN {
        return Err(errno!(EINVAL, "the socket address length is too large"));
    }

    let sun_path = &bytes[CSocketAddrUnix::PATH_OFFSET..];

    if sun_path.is_empty() {
        return Ok(UnixSocketAddr::Unnamed);
    }

    if sun_path[0] == 0 {
        return Ok(UnixSocketAddr::Abstract(Arc::from(&sun_path[1..])));
    }

    if let Ok(c_str) = CStr::from_bytes_until_nul(sun_path) {
        Ok(UnixSocketAddr::Path(Arc::from(c_str)))
    } else {
        Ok(UnixSocketAddr::Path(Arc::from(
            CString::new(sun_path).unwrap(),
        )))
    }
}
