use ip::new_ip_option;
use netlink::new_netlink_option;

use crate::{net::socket::options::SocketOption, prelude::*};

mod ip;
mod netlink;
mod socket;
mod tcp;
mod utils;

use self::{socket::new_socket_option, tcp::new_tcp_option};

pub(crate) trait RawSocketOption: SocketOption {
    fn read_from_user(&mut self, addr: Vaddr, max_len: u32) -> Result<()>;

    fn write_to_user(&self, addr: Vaddr, max_len: &mut u32) -> Result<usize>;

    fn as_sock_option_mut(&mut self) -> &mut dyn SocketOption;

    fn as_sock_option(&self) -> &dyn SocketOption;
}

macro_rules! impl_raw_socket_option {
    ($option:ty) => {
        impl RawSocketOption for $option {
            fn read_from_user(&mut self, addr: Vaddr, max_len: u32) -> Result<()> {
                use $crate::util::net::options::utils::ReadFromUser;

                let input = ReadFromUser::read_from_user(addr, max_len)?;
                self.set(input);
                Ok(())
            }

            fn write_to_user(&self, addr: Vaddr, max_len: &mut u32) -> Result<usize> {
                use $crate::util::net::options::utils::WriteToUser;

                let output = self.get().unwrap();
                output.write_to_user(addr, *max_len)
            }

            fn as_sock_option_mut(&mut self) -> &mut dyn SocketOption {
                self
            }

            fn as_sock_option(&self) -> &dyn SocketOption {
                self
            }
        }
    };
}

macro_rules! impl_raw_sock_option_get_only {
    ($option:ty) => {
        impl RawSocketOption for $option {
            fn read_from_user(&mut self, _addr: Vaddr, _max_len: u32) -> Result<()> {
                return Err(errno!(ENOPROTOOPT, "the option is getter-only"));
            }

            fn write_to_user(&self, addr: Vaddr, max_len: &mut u32) -> Result<usize> {
                use $crate::util::net::options::utils::WriteToUser;

                let output = self.get().unwrap();
                output.write_to_user(addr, *max_len)
            }

            fn as_sock_option_mut(&mut self) -> &mut dyn SocketOption {
                self
            }

            fn as_sock_option(&self) -> &dyn SocketOption {
                self
            }
        }
    };
}

use impl_raw_sock_option_get_only;
use impl_raw_socket_option;

pub(crate) fn new_raw_socket_option(
    level: CSocketOptionLevel,
    name: i32,
) -> Result<Box<dyn RawSocketOption>> {
    match level {
        CSocketOptionLevel::SOL_SOCKET => new_socket_option(name),
        CSocketOptionLevel::SOL_IP => new_ip_option(name),
        CSocketOptionLevel::SOL_TCP => new_tcp_option(name),
        CSocketOptionLevel::SOL_NETLINK => new_netlink_option(name),
        _ => Err(errno!(EOPNOTSUPP, "unsupported option level")),
    }
}

#[expect(non_camel_case_types)]
#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub(crate) enum CSocketOptionLevel {
    SOL_IP = 0,
    SOL_SOCKET = 1,
    SOL_TCP = 6,
    SOL_UDP = 17,
    SOL_IPV6 = 41,
    SOL_RAW = 255,
    SOL_NETLINK = 270,
}
