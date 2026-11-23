use alloc::{collections::btree_set::BTreeSet, sync::Arc, vec::Vec};
use core::{
    borrow::Borrow,
    sync::atomic::{AtomicU64, Ordering},
};

use smoltcp::iface::Route;

use crate::{
    ext::Ext,
    socket::{NeedIfacePoll, TcpConnectionBg},
};

pub(crate) struct PollableIface<E: Ext> {
    interface: smoltcp::iface::Interface,
    pending_conns: PendingConnSet<E>,
}

impl<E: Ext> PollableIface<E> {
    pub(super) fn new(interface: smoltcp::iface::Interface) -> Self {
        Self {
            interface,
            pending_conns: PendingConnSet::new(),
        }
    }

    pub(super) fn as_mut(&mut self) -> PollableIfaceMut<'_, E> {
        PollableIfaceMut {
            context: self.interface.context(),
            pending_conns: &mut self.pending_conns,
        }
    }

    pub(super) fn ipv4_cidr(&self) -> Option<smoltcp::wire::Ipv4Cidr> {
        self.interface.ip_addrs().iter().find_map(|cidr| {
            if let smoltcp::wire::IpCidr::Ipv4(ipv4_cidr) = cidr {
                Some(*ipv4_cidr)
            } else {
                None
            }
        })
    }

    pub(super) fn ipv6_cidr(&self) -> Option<smoltcp::wire::Ipv6Cidr> {
        self.interface.ip_addrs().iter().find_map(|cidr| {
            if let smoltcp::wire::IpCidr::Ipv6(ipv6_cidr) = cidr {
                Some(*ipv6_cidr)
            } else {
                None
            }
        })
    }

    pub(super) fn routes(&mut self) -> Vec<Route> {
        let mut routes = Vec::new();
        self.interface.routes_mut().update(|route_entries| {
            routes.extend(route_entries.iter().copied());
        });
        routes
    }

    pub(super) fn next_poll_at_ms(&self) -> Option<u64> {
        self.pending_conns.next_poll_at_ms()
    }
}

impl<E: Ext> PollableIface<E> {
    pub(crate) fn context_mut(&mut self) -> &mut smoltcp::iface::Context {
        self.interface.context()
    }

    pub(crate) fn update_next_poll_at_ms(
        &mut self,
        socket: &Arc<TcpConnectionBg<E>>,
        poll_at: smoltcp::socket::PollAt,
    ) -> NeedIfacePoll {
        self.pending_conns.update_next_poll_at_ms(socket, poll_at)
    }

    pub(crate) fn map_broadcast_to_local(
        &self,
        addr: smoltcp::wire::IpAddress,
    ) -> smoltcp::wire::IpAddress {
        use smoltcp::wire::IpAddress;

        if let IpAddress::Ipv4(addr_v4) = addr
            && let Some(cidr_v4) = self.ipv4_cidr()
            && cidr_v4.broadcast() == Some(addr_v4)
        {
            return IpAddress::Ipv4(cidr_v4.address());
        }

        addr
    }
}

pub(crate) struct PollableIfaceMut<'a, E: Ext> {
    context: &'a mut smoltcp::iface::Context,
    pending_conns: &'a mut PendingConnSet<E>,
}

impl<'a, E: Ext> PollableIfaceMut<'a, E> {
    pub(crate) fn new(
        context: &'a mut smoltcp::iface::Context,
        pending_conns: &'a mut PendingConnSet<E>,
    ) -> Self {
        Self {
            context,
            pending_conns,
        }
    }

    pub(crate) fn inner_mut(&mut self) -> (&mut smoltcp::iface::Context, &mut PendingConnSet<E>) {
        (self.context, self.pending_conns)
    }
}

impl<E: Ext> PollableIfaceMut<'_, E> {
    pub(super) fn pop_pending_tcp(&mut self) -> Option<Arc<TcpConnectionBg<E>>> {
        let now = self.context.now.total_millis() as u64;
        self.pending_conns.pop_tcp_before_now(now)
    }
}

impl<E: Ext> PollableIfaceMut<'_, E> {
    pub(crate) fn context(&self) -> &smoltcp::iface::Context {
        self.context
    }

    pub(crate) fn context_mut(&mut self) -> &mut smoltcp::iface::Context {
        self.context
    }

    pub(crate) fn update_next_poll_at_ms(
        &mut self,
        socket: &Arc<TcpConnectionBg<E>>,
        poll_at: smoltcp::socket::PollAt,
    ) -> NeedIfacePoll {
        self.pending_conns.update_next_poll_at_ms(socket, poll_at)
    }
}

pub(crate) struct PollKey {
    next_poll_at_ms: AtomicU64,
    id: usize,
}

impl PartialEq for PollKey {
    fn eq(&self, other: &Self) -> bool {
        self.next_poll_at_ms.load(Ordering::Relaxed)
            == other.next_poll_at_ms.load(Ordering::Relaxed)
            && self.id == other.id
    }
}
impl Eq for PollKey {}
impl PartialOrd for PollKey {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for PollKey {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.next_poll_at_ms
            .load(Ordering::Relaxed)
            .cmp(&other.next_poll_at_ms.load(Ordering::Relaxed))
            .then_with(|| self.id.cmp(&other.id))
    }
}

impl PollKey {
    const IMMEDIATE_VAL: u64 = 0;
    const INACTIVE_VAL: u64 = u64::MAX;

    pub(crate) fn new(id: usize) -> Self {
        Self {
            next_poll_at_ms: AtomicU64::new(Self::INACTIVE_VAL),
            id,
        }
    }

    pub(crate) fn is_active(&self) -> bool {
        self.next_poll_at_ms.load(Ordering::Relaxed) != Self::INACTIVE_VAL
    }
}

pub(crate) struct PendingConnSet<E: Ext>(BTreeSet<PendingTcpConn<E>>);

struct PendingTcpConn<E: Ext>(Arc<TcpConnectionBg<E>>);

impl<E: Ext> PartialEq for PendingTcpConn<E> {
    fn eq(&self, other: &Self) -> bool {
        self.0.poll_key() == other.0.poll_key()
    }
}
impl<E: Ext> Eq for PendingTcpConn<E> {}
impl<E: Ext> PartialOrd for PendingTcpConn<E> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<E: Ext> Ord for PendingTcpConn<E> {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.0.poll_key().cmp(other.0.poll_key())
    }
}

impl<E: Ext> Borrow<PollKey> for PendingTcpConn<E> {
    fn borrow(&self) -> &PollKey {
        self.0.poll_key()
    }
}

impl<E: Ext> PendingConnSet<E> {
    fn new() -> Self {
        Self(BTreeSet::new())
    }

    fn update_next_poll_at_ms(
        &mut self,
        socket: &Arc<TcpConnectionBg<E>>,
        poll_at: smoltcp::socket::PollAt,
    ) -> NeedIfacePoll {
        let key = socket.poll_key();
        let old_poll_at_ms = key.next_poll_at_ms.load(Ordering::Relaxed);

        let new_poll_at_ms = match poll_at {
            smoltcp::socket::PollAt::Now => PollKey::IMMEDIATE_VAL,
            smoltcp::socket::PollAt::Time(instant) => instant.total_millis() as u64,
            smoltcp::socket::PollAt::Ingress => PollKey::INACTIVE_VAL,
        };

        if old_poll_at_ms == new_poll_at_ms {
            return NeedIfacePoll::FALSE;
        }

        let owned_socket = if old_poll_at_ms != PollKey::INACTIVE_VAL {
            self.0.take(key).unwrap()
        } else {
            PendingTcpConn(socket.clone())
        };

        key.next_poll_at_ms.store(new_poll_at_ms, Ordering::Relaxed);

        if new_poll_at_ms == PollKey::INACTIVE_VAL {
            return NeedIfacePoll::FALSE;
        }

        let inserted = self.0.insert(owned_socket);
        debug_assert!(inserted);

        if new_poll_at_ms < old_poll_at_ms {
            NeedIfacePoll::TRUE
        } else {
            NeedIfacePoll::FALSE
        }
    }

    fn pop_tcp_before_now(&mut self, now_at_ms: u64) -> Option<Arc<TcpConnectionBg<E>>> {
        if self.0.first().is_some_and(|first| {
            first.0.poll_key().next_poll_at_ms.load(Ordering::Relaxed) <= now_at_ms
        }) {
            self.0.pop_first().map(|first| {
                first
                    .0
                    .poll_key()
                    .next_poll_at_ms
                    .store(PollKey::INACTIVE_VAL, Ordering::Relaxed);
                first.0
            })
        } else {
            None
        }
    }

    fn next_poll_at_ms(&self) -> Option<u64> {
        self.0
            .first()
            .map(|first| first.0.poll_key().next_poll_at_ms.load(Ordering::Relaxed))
    }
}

pub(super) trait IsUnicast {
    fn is_unicast(&self, dst_addr: smoltcp::wire::IpAddress) -> bool;

    fn is_unicast_local(&self, dst_addr: smoltcp::wire::IpAddress) -> bool;
}

impl IsUnicast for smoltcp::iface::Context {
    fn is_unicast(&self, dst_addr: smoltcp::wire::IpAddress) -> bool {
        !self.is_broadcast(&dst_addr) && !dst_addr.is_multicast()
    }

    fn is_unicast_local(&self, dst_addr: smoltcp::wire::IpAddress) -> bool {
        use smoltcp::wire::IpAddress;

        match dst_addr {
            IpAddress::Ipv4(dst_addr) => self.ipv4_addr().is_some_and(|addr| {
                addr == dst_addr || (addr.is_loopback() && dst_addr.is_loopback())
            }),
            IpAddress::Ipv6(dst_addr) => self.ipv6_addr().is_some_and(|addr| addr == dst_addr),
        }
    }
}
