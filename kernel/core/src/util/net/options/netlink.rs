use ostd::mm::VmIo;

use super::{RawSocketOption, SocketOption};
use crate::{
    context::current_userspace,
    net::socket::netlink::{AddMembership, DropMembership, StrictCheck},
    prelude::*,
};

#[expect(non_camel_case_types)]
#[expect(clippy::upper_case_acronyms)]
#[repr(i32)]
#[derive(Clone, Copy, Debug, TryFromInt)]
pub(crate) enum CNetlinkOptionName {
    ADD_MEMBERSHIP = 1,
    DROP_MEMBERSHIP = 2,
    PKTINFO = 3,
    GET_STRICT_CHK = 12,
}

pub(crate) fn new_netlink_option(name: i32) -> Result<Box<dyn RawSocketOption>> {
    let name = CNetlinkOptionName::try_from(name).map_err(|_| Errno::ENOPROTOOPT)?;
    match name {
        CNetlinkOptionName::ADD_MEMBERSHIP => Ok(Box::new(AddMembership::new())),
        CNetlinkOptionName::DROP_MEMBERSHIP => Ok(Box::new(DropMembership::new())),
        CNetlinkOptionName::GET_STRICT_CHK => Ok(Box::new(StrictCheck::new())),
        _ => Err(errno!(ENOPROTOOPT, "unsupported netlink option")),
    }
}

trait ReadNetlinkOption: Sized {
    fn read_netlink_option(addr: Vaddr, max_len: u32) -> Result<Self>;
}

macro_rules! impl_raw_netlink_socket_option {
    ($option:ty) => {
        impl RawSocketOption for $option {
            fn read_from_user(&mut self, addr: Vaddr, max_len: u32) -> Result<()> {
                let input = ReadNetlinkOption::read_netlink_option(addr, max_len)?;
                self.set(input);
                Ok(())
            }

            fn write_to_user(&self, addr: Vaddr, max_len: &mut u32) -> Result<usize> {
                use $crate::util::net::options::utils::WriteToUser;

                if *max_len < size_of::<u32>() as u32 {
                    return Err(errno!(EINVAL, "max_len is too short"));
                }

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

macro_rules! impl_raw_netlink_sock_option_set_only {
    ($option:ty) => {
        impl RawSocketOption for $option {
            fn read_from_user(&mut self, addr: Vaddr, max_len: u32) -> Result<()> {
                let input = ReadNetlinkOption::read_netlink_option(addr, max_len)?;
                self.set(input);
                Ok(())
            }

            fn write_to_user(&self, _addr: Vaddr, _max_len: &mut u32) -> Result<usize> {
                return Err(errno!(ENOPROTOOPT, "the option is setter-only"));
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

impl_raw_netlink_sock_option_set_only!(AddMembership);
impl_raw_netlink_sock_option_set_only!(DropMembership);
impl_raw_netlink_socket_option!(StrictCheck);

impl ReadNetlinkOption for u32 {
    fn read_netlink_option(addr: Vaddr, max_len: u32) -> Result<Self> {
        if max_len >= size_of::<u32>() as u32 {
            Ok(current_userspace!().read_val::<u32>(addr)?)
        } else {
            Ok(0)
        }
    }
}

impl ReadNetlinkOption for bool {
    fn read_netlink_option(addr: Vaddr, max_len: u32) -> Result<Self> {
        Ok(u32::read_netlink_option(addr, max_len)? != 0)
    }
}
