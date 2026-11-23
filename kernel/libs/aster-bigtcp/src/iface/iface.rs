use alloc::{sync::Arc, vec::Vec};

use smoltcp::{
    iface::Route,
    wire::{EthernetAddress, Ipv4Address, Ipv4Cidr, Ipv6Cidr},
};

use super::{
    BindPortConfig, BoundTcpPort, BoundUdpPort, InterfaceFlags, InterfaceName, InterfaceType,
};
use crate::{errors::BindError, ext::Ext};

pub trait Iface<E>: internal::IfaceInternal<E> + Send + Sync {
    fn ethernet_addr(&self) -> Option<EthernetAddress>;

    fn poll(&self);

    fn mtu(&self) -> usize;
}

impl<E: Ext> dyn Iface<E> {
    pub fn bind_tcp(
        self: &Arc<Self>,
        config: BindPortConfig,
    ) -> Result<BoundTcpPort<E>, BindError> {
        let common = self.common();
        common.bind_tcp(self.clone(), config)
    }

    pub fn bind_udp(
        self: &Arc<Self>,
        config: BindPortConfig,
    ) -> Result<BoundUdpPort<E>, BindError> {
        let common = self.common();
        common.bind_udp(self.clone(), config)
    }

    pub fn index(&self) -> u32 {
        self.common().index()
    }

    pub fn name(&self) -> &InterfaceName {
        self.common().name()
    }

    pub fn type_(&self) -> InterfaceType {
        self.common().type_()
    }

    pub fn flags(&self) -> InterfaceFlags {
        self.common().flags()
    }

    pub fn ipv4_cidr(&self) -> Option<Ipv4Cidr> {
        self.common().ipv4_cidr()
    }

    pub fn ipv6_cidr(&self) -> Option<Ipv6Cidr> {
        self.common().ipv6_cidr()
    }

    pub fn broadcast_addr(&self) -> Option<Ipv4Address> {
        self.common().ipv4_cidr()?.broadcast()
    }

    pub fn routes(&self) -> Vec<Route> {
        self.common().routes()
    }

    pub fn sched_poll(&self) -> &E::ScheduleNextPoll {
        self.common().sched_poll()
    }
}

pub(super) mod internal {
    use crate::{ext::Ext, iface::common::IfaceCommon};

    pub trait IfaceInternal<E> {
        fn common(&self) -> &IfaceCommon<E>
        where
            E: Ext;
    }
}
