use aster_bigtcp::{
    iface::InterfaceType,
    wire::{IpAddress, IpCidr, IpEndpoint, Ipv4Address, Ipv4Cidr, Ipv6Address, Ipv6Cidr},
};
use spin::Once;

use self::{manager::RouteManager, rule::Rule};
use super::iface::{self, Iface};
use crate::prelude::*;

mod entry;
mod manager;
mod rule;
mod table;

use entry::{RouteAddressFamily, RouteEntry, RouteMetric, RouteTableId, RouteType};

static IPV4_ROUTE_MANAGER: Once<RouteManager<Ipv4Address>> = Once::new();
static IPV6_ROUTE_MANAGER: Once<RouteManager<Ipv6Address>> = Once::new();

#[derive(Clone, Debug)]
struct RouteLookupKey<A: RouteAddressFamily> {
    dst: A,
}

impl<A: RouteAddressFamily> RouteLookupKey<A> {
    const fn new(dst: A) -> Self {
        Self { dst }
    }

    const fn dst(&self) -> &A {
        &self.dst
    }
}

pub(super) fn init() {
    const LOCAL_RULE_PRIORITY: u32 = 0;
    const MAIN_RULE_PRIORITY: u32 = 32766;
    const DEFAULT_RULE_PRIORITY: u32 = 32767;

    IPV4_ROUTE_MANAGER.call_once(|| {
        let routes = iface::iter_all_ifaces()
            .flat_map(iface_ipv4_routes)
            .collect();
        RouteManager::new(
            &[
                Rule::lookup(LOCAL_RULE_PRIORITY, RouteTableId::LOCAL),
                Rule::lookup(MAIN_RULE_PRIORITY, RouteTableId::MAIN),
                Rule::lookup(DEFAULT_RULE_PRIORITY, RouteTableId::DEFAULT),
            ],
            routes,
        )
    });

    IPV6_ROUTE_MANAGER.call_once(|| {
        let routes = iface::iter_all_ifaces()
            .flat_map(iface_ipv6_routes)
            .collect();
        RouteManager::new(
            &[
                Rule::lookup(LOCAL_RULE_PRIORITY, RouteTableId::LOCAL),
                Rule::lookup(MAIN_RULE_PRIORITY, RouteTableId::MAIN),
            ],
            routes,
        )
    });
}

fn iface_ipv4_routes(iface: &Arc<Iface>) -> Vec<RouteEntry<Ipv4Address>> {
    let mut routes = Vec::new();

    if let Some(iface_cidr) = iface.ipv4_cidr() {
        let iface_addr = iface_cidr.address();

        let entry = RouteEntry::new(
            Ipv4Cidr::new(iface_addr, 32),
            None,
            iface.clone(),
            RouteTableId::LOCAL,
            RouteMetric::default(),
            RouteType::Local,
        )
        .unwrap();
        routes.push(entry);

        if iface_cidr.prefix_len() < 32 {
            let (table_id, type_) = if iface.type_() == InterfaceType::LOOPBACK {
                (RouteTableId::LOCAL, RouteType::Local)
            } else {
                (RouteTableId::MAIN, RouteType::Unicast)
            };
            let entry = RouteEntry::new(
                iface_cidr.network(),
                None,
                iface.clone(),
                table_id,
                RouteMetric::default(),
                type_,
            )
            .unwrap();
            routes.push(entry);
        }

        if let Some(broadcast_addr) = iface.broadcast_addr() {
            let entry = RouteEntry::new(
                Ipv4Cidr::new(broadcast_addr, 32),
                None,
                iface.clone(),
                RouteTableId::LOCAL,
                RouteMetric::default(),
                RouteType::Broadcast,
            )
            .unwrap();
            routes.push(entry);
        }
    }

    for route in iface.routes() {
        let (IpCidr::Ipv4(dst), IpAddress::Ipv4(gateway)) = (route.cidr, route.via_router) else {
            continue;
        };
        let entry = RouteEntry::new(
            dst,
            Some(gateway),
            iface.clone(),
            RouteTableId::MAIN,
            RouteMetric::default(),
            RouteType::Unicast,
        )
        .unwrap();
        routes.push(entry);
    }

    routes
}

fn iface_ipv6_routes(iface: &Arc<Iface>) -> Vec<RouteEntry<Ipv6Address>> {
    let mut routes = Vec::new();

    let Some(iface_cidr) = iface.ipv6_cidr() else {
        return routes;
    };

    let entry = RouteEntry::new(
        Ipv6Cidr::new(iface_cidr.address(), 128),
        None,
        iface.clone(),
        RouteTableId::LOCAL,
        RouteMetric::default(),
        RouteType::Local,
    )
    .unwrap();
    routes.push(entry);

    routes
}

pub(super) fn lookup_iface(dst: IpAddress) -> Result<Arc<Iface>> {
    let iface = match dst {
        IpAddress::Ipv4(dst) => IPV4_ROUTE_MANAGER
            .get()
            .unwrap()
            .lookup_entry(&RouteLookupKey::new(dst))?
            .output_iface()
            .clone(),
        IpAddress::Ipv6(dst) => IPV6_ROUTE_MANAGER
            .get()
            .unwrap()
            .lookup_entry(&RouteLookupKey::new(dst))?
            .output_iface()
            .clone(),
    };
    Ok(iface)
}

pub(super) fn lookup_local_iface(address: IpAddress) -> Result<Arc<Iface>> {
    let (type_, iface) = match address {
        IpAddress::Ipv4(address) => {
            let entry = IPV4_ROUTE_MANAGER
                .get()
                .unwrap()
                .lookup_in_local_table(&RouteLookupKey::new(address))
                .ok_or_else(|| {
                    errno!(
                        EADDRNOTAVAIL,
                        "the address is not available from the local machine",
                    )
                })?;
            (entry.type_(), entry.output_iface().clone())
        }
        IpAddress::Ipv6(address) => {
            let entry = IPV6_ROUTE_MANAGER
                .get()
                .unwrap()
                .lookup_in_local_table(&RouteLookupKey::new(address))
                .ok_or_else(|| {
                    errno!(
                        EADDRNOTAVAIL,
                        "the address is not available from the local machine",
                    )
                })?;
            (entry.type_(), entry.output_iface().clone())
        }
    };

    if type_ != RouteType::Local && type_ != RouteType::Broadcast {
        return Err(errno!(
            EADDRNOTAVAIL,
            "the address is not available from the local machine"
        ));
    }
    Ok(iface)
}

pub(super) fn is_broadcast_endpoint(endpoint: &IpEndpoint) -> bool {
    let IpAddress::Ipv4(address) = endpoint.addr else {
        return false;
    };

    address.is_broadcast()
        || IPV4_ROUTE_MANAGER
            .get()
            .unwrap()
            .lookup_entry(&RouteLookupKey::new(address))
            .is_ok_and(|entry| entry.type_() == RouteType::Broadcast)
}
