use aster_bigtcp::wire::{IpAddress, IpEndpoint, Ipv4Address, Ipv6Address};

use crate::{net::socket::util::SocketAddr, prelude::*};

impl TryFrom<SocketAddr> for IpEndpoint {
    type Error = Error;

    fn try_from(value: SocketAddr) -> Result<Self> {
        match value {
            SocketAddr::IPv4(addr, port) => Ok(IpEndpoint::new(addr.into(), port)),
            SocketAddr::IPv6(addr, port) => Ok(IpEndpoint::new(addr.into(), port)),
            _ => Err(errno!(
                EAFNOSUPPORT,
                "the address is in an unsupported address family"
            )),
        }
    }
}

impl From<IpEndpoint> for SocketAddr {
    fn from(endpoint: IpEndpoint) -> Self {
        let port = endpoint.port;
        match endpoint.addr {
            IpAddress::Ipv4(addr) => SocketAddr::IPv4(addr, port),
            IpAddress::Ipv6(addr) => SocketAddr::IPv6(addr, port),
        }
    }
}

pub(super) const UNSPECIFIED_LOCAL_ENDPOINT: IpEndpoint =
    IpEndpoint::new(IpAddress::Ipv4(Ipv4Address::UNSPECIFIED), 0);

pub(super) const UNSPECIFIED_LOCAL_ENDPOINT_V6: IpEndpoint =
    IpEndpoint::new(IpAddress::Ipv6(Ipv6Address::UNSPECIFIED), 0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IpAddressFamily {
    IPv4,
    IPv6,
}

impl IpAddressFamily {
    pub(super) const fn unspecified_endpoint(&self) -> IpEndpoint {
        match self {
            IpAddressFamily::IPv4 => UNSPECIFIED_LOCAL_ENDPOINT,
            IpAddressFamily::IPv6 => UNSPECIFIED_LOCAL_ENDPOINT_V6,
        }
    }
}

impl From<IpAddress> for IpAddressFamily {
    fn from(addr: IpAddress) -> Self {
        match addr {
            IpAddress::Ipv4(_) => IpAddressFamily::IPv4,
            IpAddress::Ipv6(_) => IpAddressFamily::IPv6,
        }
    }
}
