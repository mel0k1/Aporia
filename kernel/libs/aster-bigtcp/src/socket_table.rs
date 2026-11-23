use alloc::{boxed::Box, sync::Arc, vec::Vec};
use core::net::Ipv4Addr;

use jhash::{jhash_1vals, jhash_3vals, jhash_u32_array};
use ostd::const_assert;
use smoltcp::wire::{IpAddress, IpEndpoint, IpListenEndpoint};

use crate::{
    ext::Ext,
    socket::{TcpConnectionBg, TcpListenerBg, UdpSocketBg},
    wire::PortNum,
};

pub type SocketHash = u32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ListenerKey {
    addr: IpAddress,
    port: PortNum,
    hash: SocketHash,
}

impl ListenerKey {
    pub(crate) const fn new(addr: IpAddress, port: PortNum) -> Self {
        let hash = hash_addr_port(addr, port);
        Self { addr, port, hash }
    }

    pub(crate) const fn hash(&self) -> SocketHash {
        self.hash
    }
}

impl From<IpListenEndpoint> for ListenerKey {
    fn from(listen_endpoint: IpListenEndpoint) -> Self {
        let addr = listen_endpoint
            .addr
            .unwrap_or(IpAddress::Ipv4(Ipv4Addr::UNSPECIFIED));
        let port = listen_endpoint.port;
        Self::new(addr, port)
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct ConnectionKey {
    local_addr: IpAddress,
    local_port: PortNum,
    remote_addr: IpAddress,
    remote_port: PortNum,
    hash: SocketHash,
}

impl ConnectionKey {
    pub(crate) const fn new(
        local_addr: IpAddress,
        local_port: PortNum,
        remote_addr: IpAddress,
        remote_port: PortNum,
    ) -> Self {
        let hash = hash_local_remote(local_addr, local_port, remote_addr, remote_port);
        Self {
            local_addr,
            local_port,
            remote_addr,
            remote_port,
            hash,
        }
    }

    pub(crate) const fn hash(&self) -> SocketHash {
        self.hash
    }

    pub(crate) const fn remote_endpoint(&self) -> IpEndpoint {
        IpEndpoint::new(self.remote_addr, self.remote_port)
    }
}

impl From<(IpEndpoint, IpEndpoint)> for ConnectionKey {
    fn from(value: (IpEndpoint, IpEndpoint)) -> Self {
        Self::new(value.0.addr, value.0.port, value.1.addr, value.1.port)
    }
}

const HASH_SECRET: u32 = 0xdeadbeef;

const NET_HASHMIX: u32 = 0xbeefdead;

const fn hash_local_remote(
    local_addr: IpAddress,
    local_port: PortNum,
    remote_addr: IpAddress,
    remote_port: PortNum,
) -> SocketHash {
    match (local_addr, remote_addr) {
        (IpAddress::Ipv4(local_ipv4), IpAddress::Ipv4(remote_ipv4)) => jhash_3vals(
            local_ipv4.to_bits(),
            remote_ipv4.to_bits(),
            (local_port as u32).wrapping_shl(16) | remote_port as u32,
            HASH_SECRET.wrapping_add(NET_HASHMIX),
        ),
        (IpAddress::Ipv6(local_ipv6), IpAddress::Ipv6(remote_ipv6)) => {
            let local_bits = local_ipv6.to_bits();
            let remote_bits = remote_ipv6.to_bits();
            let hash_keys = &[
                (local_bits >> 96) as u32,
                (local_bits >> 64) as u32,
                (local_bits >> 32) as u32,
                local_bits as u32,
                (remote_bits >> 96) as u32,
                (remote_bits >> 64) as u32,
                (remote_bits >> 32) as u32,
                remote_bits as u32,
                (local_port as u32).wrapping_shl(16) | remote_port as u32,
            ];
            jhash_u32_array(hash_keys, HASH_SECRET.wrapping_add(NET_HASHMIX))
        }
        _ => panic!("cannot mix IPv4 and IPv6 addresses"),
    }
}

const fn hash_addr_port(addr: IpAddress, port: PortNum) -> SocketHash {
    match addr {
        IpAddress::Ipv4(ipv4_addr) => jhash_1vals(ipv4_addr.to_bits(), NET_HASHMIX) ^ (port as u32),
        IpAddress::Ipv6(ipv6_addr) => {
            let bits = ipv6_addr.to_bits();
            let hash_keys = &[
                (bits >> 96) as u32,
                (bits >> 64) as u32,
                (bits >> 32) as u32,
                bits as u32,
                port as u32,
            ];
            jhash_u32_array(hash_keys, NET_HASHMIX)
        }
    }
}

pub(crate) struct SocketTable<E: Ext> {
    listener_buckets: Box<[ListenerHashBucket<E>]>,
    connection_buckets: Box<[ConnectionHashBucket<E>]>,
    udp_sockets: Vec<Arc<UdpSocketBg<E>>>,
}

const LISTENER_BUCKET_COUNT: u32 = 64;
const LISTENER_BUCKET_MASK: u32 = LISTENER_BUCKET_COUNT - 1;
const CONNECTION_BUCKET_COUNT: u32 = 8192;
const CONNECTION_BUCKET_MASK: u32 = CONNECTION_BUCKET_COUNT - 1;

const_assert!(LISTENER_BUCKET_COUNT.is_power_of_two());
const_assert!(CONNECTION_BUCKET_COUNT.is_power_of_two());

impl<E: Ext> SocketTable<E> {
    pub(crate) fn new() -> Self {
        let listener_buckets = (0..LISTENER_BUCKET_COUNT)
            .map(|_| ListenerHashBucket::new())
            .collect();

        let connection_buckets = (0..CONNECTION_BUCKET_COUNT)
            .map(|_| ConnectionHashBucket::new())
            .collect();

        let udp_sockets = Vec::new();

        Self {
            listener_buckets,
            connection_buckets,
            udp_sockets,
        }
    }

    pub(crate) fn insert_listener(
        &mut self,
        listener: Arc<TcpListenerBg<E>>,
    ) -> Result<(), Arc<TcpListenerBg<E>>> {
        let key = listener.listener_key();

        let bucket = {
            let hash = key.hash();
            let bucket_index = hash & LISTENER_BUCKET_MASK;
            &mut self.listener_buckets[bucket_index as usize]
        };

        if bucket
            .listeners
            .iter()
            .any(|tcp_listener| tcp_listener.listener_key() == listener.listener_key())
        {
            return Err(listener);
        }

        bucket.listeners.push(listener);
        Ok(())
    }

    pub(crate) fn insert_connection(
        &mut self,
        connection: Arc<TcpConnectionBg<E>>,
    ) -> Result<(), Arc<TcpConnectionBg<E>>> {
        let key = connection.connection_key();

        let bucket = {
            let hash = key.hash();
            let bucket_index = hash & CONNECTION_BUCKET_MASK;
            &mut self.connection_buckets[bucket_index as usize]
        };

        if bucket
            .connections
            .iter()
            .any(|tcp_connection| tcp_connection.connection_key() == connection.connection_key())
        {
            return Err(connection);
        }

        bucket.connections.push(connection);
        Ok(())
    }

    pub(crate) fn insert_udp_socket(&mut self, udp_socket: Arc<UdpSocketBg<E>>) {
        debug_assert!(
            !self
                .udp_sockets
                .iter()
                .any(|socket| Arc::ptr_eq(socket, &udp_socket))
        );
        self.udp_sockets.push(udp_socket);
    }

    pub(crate) fn lookup_listener(&self, key: &ListenerKey) -> Option<&Arc<TcpListenerBg<E>>> {
        let bucket = {
            let hash = key.hash();
            let bucket_index = hash & LISTENER_BUCKET_MASK;
            &self.listener_buckets[bucket_index as usize]
        };

        bucket
            .listeners
            .iter()
            .find(|listener| listener.listener_key() == key)
    }

    pub(crate) fn lookup_connection(
        &self,
        key: &ConnectionKey,
    ) -> Option<&Arc<TcpConnectionBg<E>>> {
        let bucket = {
            let hash = key.hash();
            let bucket_index = hash & CONNECTION_BUCKET_MASK;
            &self.connection_buckets[bucket_index as usize]
        };

        bucket
            .connections
            .iter()
            .find(|connection| connection.connection_key() == key)
    }

    pub(crate) fn remove_listener(&mut self, key: &ListenerKey) -> Option<Arc<TcpListenerBg<E>>> {
        let bucket = {
            let hash = key.hash();
            let bucket_index = hash & LISTENER_BUCKET_MASK;
            &mut self.listener_buckets[bucket_index as usize]
        };

        let index = bucket
            .listeners
            .iter()
            .position(|tcp_listener| tcp_listener.listener_key() == key)?;
        Some(bucket.listeners.swap_remove(index))
    }

    pub(crate) fn remove_dead_tcp_connection(&mut self, key: &ConnectionKey) {
        let bucket = {
            let hash = key.hash();
            let bucket_index = hash & CONNECTION_BUCKET_MASK;
            &mut self.connection_buckets[bucket_index as usize]
        };

        let index = bucket
            .connections
            .iter()
            .position(|tcp_connection| tcp_connection.connection_key() == key)
            .unwrap();
        let connection = bucket.connections.swap_remove(index);
        debug_assert!(
            !connection.poll_key().is_active(),
            "there should be no need to poll a dead TCP connection",
        );

        connection.notify_dead_events();
    }

    pub(crate) fn remove_udp_socket(
        &mut self,
        socket: &Arc<UdpSocketBg<E>>,
    ) -> Option<Arc<UdpSocketBg<E>>> {
        let index = self
            .udp_sockets
            .iter()
            .position(|udp_socket| Arc::ptr_eq(udp_socket, socket))?;
        Some(self.udp_sockets.swap_remove(index))
    }

    pub(crate) fn udp_socket_iter(&self) -> impl Iterator<Item = &Arc<UdpSocketBg<E>>> {
        self.udp_sockets.iter()
    }
}

impl<E: Ext> Default for SocketTable<E> {
    fn default() -> Self {
        Self::new()
    }
}

struct ListenerHashBucket<E: Ext> {
    listeners: Vec<Arc<TcpListenerBg<E>>>,
}

impl<E: Ext> ListenerHashBucket<E> {
    const fn new() -> Self {
        Self {
            listeners: Vec::new(),
        }
    }
}

struct ConnectionHashBucket<E: Ext> {
    connections: Vec<Arc<TcpConnectionBg<E>>>,
}

impl<E: Ext> ConnectionHashBucket<E> {
    const fn new() -> Self {
        Self {
            connections: Vec::new(),
        }
    }
}
