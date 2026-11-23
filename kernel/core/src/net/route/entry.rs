use aster_bigtcp::wire::{Ipv4Address, Ipv4Cidr, Ipv6Address, Ipv6AddressExt, Ipv6Cidr};

use super::RouteLookupKey;
use crate::{net::iface::Iface, prelude::*};

#[derive(Clone)]
pub(super) struct RouteEntry<A: RouteAddressFamily> {
    dst: A::Cidr,
    #[expect(dead_code)]
    gateway: Option<A>,
    output_iface: Arc<Iface>,
    table_id: RouteTableId,
    metric: RouteMetric,
    type_: RouteType,
}

impl<A: RouteAddressFamily> RouteEntry<A> {
    pub(super) fn new(
        dst: A::Cidr,
        gateway: Option<A>,
        output_iface: Arc<Iface>,
        table_id: RouteTableId,
        metric: RouteMetric,
        type_: RouteType,
    ) -> Result<Self> {
        if dst != A::network(&dst) {
            return Err(errno!(EINVAL, "the route destination is not canonical"));
        }

        Ok(Self {
            dst,
            gateway,
            output_iface,
            table_id,
            metric,
            type_,
        })
    }

    pub(super) const fn dst(&self) -> &A::Cidr {
        &self.dst
    }

    #[expect(dead_code)]
    const fn gateway(&self) -> Option<A> {
        self.gateway
    }

    pub(super) const fn output_iface(&self) -> &Arc<Iface> {
        &self.output_iface
    }

    pub(super) const fn table_id(&self) -> RouteTableId {
        self.table_id
    }

    pub(super) const fn metric(&self) -> RouteMetric {
        self.metric
    }

    pub(super) const fn type_(&self) -> RouteType {
        self.type_
    }

    pub(super) fn matches_lookup(&self, key: &RouteLookupKey<A>) -> bool {
        A::contains(&self.dst, key.dst())
    }
}

pub(super) trait RouteAddressFamily: Copy + Eq + Ord {
    type Cidr: Copy + Eq + Ord;

    fn contains(cidr: &Self::Cidr, address: &Self) -> bool;

    fn network(cidr: &Self::Cidr) -> Self::Cidr;

    fn prefix_len(cidr: &Self::Cidr) -> u8;
}

impl RouteAddressFamily for Ipv4Address {
    type Cidr = Ipv4Cidr;

    fn contains(cidr: &Self::Cidr, address: &Self) -> bool {
        cidr.contains_addr(address)
    }

    fn network(cidr: &Self::Cidr) -> Self::Cidr {
        cidr.network()
    }

    fn prefix_len(cidr: &Self::Cidr) -> u8 {
        cidr.prefix_len()
    }
}

impl RouteAddressFamily for Ipv6Address {
    type Cidr = Ipv6Cidr;

    fn contains(cidr: &Self::Cidr, address: &Self) -> bool {
        cidr.contains_addr(address)
    }

    fn network(cidr: &Self::Cidr) -> Self::Cidr {
        Ipv6Cidr::new(
            cidr.address().mask(cidr.prefix_len()).into(),
            cidr.prefix_len(),
        )
    }

    fn prefix_len(cidr: &Self::Cidr) -> u8 {
        cidr.prefix_len()
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct RouteTableId(u32);

impl RouteTableId {
    pub(super) const DEFAULT: Self = Self(253);
    pub(super) const MAIN: Self = Self(254);
    pub(super) const LOCAL: Self = Self(255);
}

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct RouteMetric(u32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RouteType {
    Unicast,
    Local,
    Broadcast,
}
