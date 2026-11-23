use core::cmp::min;

use ostd::{mm::VmIo, task::Task};

use super::{
    ip::{CSocketAddrInet, CSocketAddrInet6},
    netlink::CSocketAddrNetlink,
    unix,
    vsock::CSocketAddrVm,
};
use crate::{context::current_userspace, net::socket::util::SocketAddr, prelude::*};

#[expect(non_camel_case_types)]
#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub(crate) enum CSocketAddrFamily {
    AF_UNSPEC = 0,
    AF_UNIX = 1,
    AF_INET = 2,
    AF_AX25 = 3,
    AF_IPX = 4,
    AF_APPLETALK = 5,
    AF_NETROM = 6,
    AF_BRIDGE = 7,
    AF_ATMPVC = 8,
    AF_X25 = 9,
    AF_INET6 = 10,
    AF_ROSE = 11,
    AF_DECnet = 12,
    AF_NETBEUI = 13,
    AF_SECURITY = 14,
    AF_KEY = 15,
    AF_NETLINK = 16,
    AF_PACKET = 17,
    AF_ASH = 18,
    AF_ECONET = 19,
    AF_ATMSVC = 20,
    AF_RDS = 21,
    AF_SNA = 22,
    AF_IRDA = 23,
    AF_PPPOX = 24,
    AF_WANPIPE = 25,
    AF_LLC = 26,
    AF_IB = 27,
    AF_MPLS = 28,
    AF_CAN = 29,
    AF_TIPC = 30,
    AF_BLUETOOTH = 31,
    AF_IUCV = 32,
    AF_RXRPC = 33,
    AF_ISDN = 34,
    AF_PHONET = 35,
    AF_IEEE802154 = 36,
    AF_CAIF = 37,
    AF_ALG = 38,
    AF_NFC = 39,
    AF_VSOCK = 40,
    AF_KCM = 41,
    AF_QIPCRTR = 42,
    AF_SMC = 43,
    AF_XDP = 44,
    AF_MCTP = 45,
}

const ADDR_MAX_LEN: usize = 128;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct Storage {
    sa_family: u16,
    bytes: [u8; ADDR_MAX_LEN - 2],
    _align: [u64; 0],
}

pub(crate) fn read_socket_addr_from_user(addr: Vaddr, addr_len: usize) -> Result<SocketAddr> {
    if addr_len > ADDR_MAX_LEN {
        return Err(errno!(EINVAL, "the socket address length is too large"));
    }

    if addr_len < 2 {
        return Err(errno!(EINVAL, "the socket address length is too small"));
    }

    let mut storage = Storage::new_zeroed();
    current_userspace!().read_bytes(addr, &mut storage.as_mut_bytes()[..addr_len])?;

    let result = match CSocketAddrFamily::try_from(storage.sa_family as i32) {
        Ok(CSocketAddrFamily::AF_INET) => {
            if addr_len < size_of::<CSocketAddrInet>() {
                return Err(errno!(EINVAL, "the socket address length is too small"));
            }
            let (addr, port) = CSocketAddrInet::from_first_bytes(storage.as_bytes()).into();
            SocketAddr::IPv4(addr, port)
        }
        Ok(CSocketAddrFamily::AF_INET6) => {
            if addr_len < size_of::<CSocketAddrInet6>() {
                return Err(errno!(EINVAL, "the socket address length is too small"));
            }
            let (addr, port) = CSocketAddrInet6::from_first_bytes(storage.as_bytes()).into();
            SocketAddr::IPv6(addr, port)
        }
        Ok(CSocketAddrFamily::AF_UNIX) => {
            let addr = unix::from_c_bytes(&storage.as_bytes()[..addr_len])?;
            SocketAddr::Unix(addr)
        }
        Ok(CSocketAddrFamily::AF_NETLINK) => {
            if addr_len < size_of::<CSocketAddrNetlink>() {
                return Err(errno!(EINVAL, "the socket address length is too small"));
            }
            let addr = CSocketAddrNetlink::from_first_bytes(storage.as_bytes());
            SocketAddr::Netlink(addr.into())
        }
        Ok(CSocketAddrFamily::AF_VSOCK) => {
            if addr_len < size_of::<CSocketAddrVm>() {
                return Err(errno!(EINVAL, "the socket address length is too small"));
            }
            let addr = CSocketAddrVm::from_first_bytes(storage.as_bytes());
            SocketAddr::Vsock(addr.into())
        }
        _ => {
            return Err(errno!(
                EAFNOSUPPORT,
                "the specified address family is not supported"
            ));
        }
    };

    Ok(result)
}

pub(crate) fn write_socket_addr_to_user(
    socket_addr: &SocketAddr,
    dest: Vaddr,
    max_len_ptr: Vaddr,
) -> Result<()> {
    let current_task = Task::current().unwrap();
    let user_space = CurrentUserSpace::new(current_task.as_thread_local().unwrap());

    let max_len = user_space.read_val::<i32>(max_len_ptr)?;

    let actual_len = write_socket_addr_with_max_len(socket_addr, dest, max_len)?;

    Ok(user_space.write_val(max_len_ptr, &actual_len)?)
}

pub(crate) fn write_socket_addr_with_max_len(
    socket_addr: &SocketAddr,
    dest: Vaddr,
    max_len: i32,
) -> Result<i32> {
    if max_len < 0 {
        return Err(errno!(
            EINVAL,
            "the socket address length cannot be negative"
        ));
    }

    let actual_len = match socket_addr {
        SocketAddr::IPv4(addr, port) => write_c_socket_address_util::<CSocketAddrInet, _>(
            (*addr, *port),
            dest,
            max_len as usize,
        )?,
        SocketAddr::IPv6(addr, port) => write_c_socket_address_util::<CSocketAddrInet6, _>(
            (*addr, *port),
            dest,
            max_len as usize,
        )?,
        SocketAddr::Unix(addr) => unix::into_c_bytes_and(addr, |bytes| {
            let written_len = min(bytes.len(), max_len as _);
            current_userspace!().write_bytes(dest, &bytes[..written_len])?;
            Ok::<usize, Error>(bytes.len())
        })?,
        SocketAddr::Netlink(addr) => {
            write_c_socket_address_util::<CSocketAddrNetlink, _>(*addr, dest, max_len as usize)?
        }
        SocketAddr::Vsock(addr) => {
            write_c_socket_address_util::<CSocketAddrVm, _>(*addr, dest, max_len as usize)?
        }
    };

    Ok(actual_len as i32)
}

fn write_c_socket_address_util<TCSockAddr, TSockAddr>(
    addr: TSockAddr,
    dest: Vaddr,
    max_len: usize,
) -> Result<usize>
where
    TCSockAddr: Pod + From<TSockAddr>,
{
    let c_socket_addr = TCSockAddr::from(addr);
    let actual_len = size_of::<TCSockAddr>();
    let written_len = min(actual_len, max_len);

    current_userspace!().write_bytes(dest, &c_socket_addr.as_bytes()[..written_len])?;

    Ok(actual_len)
}
