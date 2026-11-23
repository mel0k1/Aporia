use aster_softirq::BottomHalfDisabled;
use aster_virtio::device::socket::{
    device::SocketDevice,
    header::{VirtioVsockHdr, VirtioVsockOp, VirtioVsockShutdownFlags, VirtioVsockType},
    packet::{RxPacket, TxPacket},
};
use spin::Once;

use crate::{
    events::IoEvents,
    net::socket::vsock::{
        addr::VsockSocketAddr,
        transport::{
            BoundPort, Connection, Listener, conn_id::ConnId, connection::ConnectionInner,
            listener::ListenerInner, port::PortTable, timer::TimerEvent,
        },
    },
    prelude::*,
    process::signal::Pollee,
};

pub(super) struct VsockSpace {
    device: Arc<SocketDevice>,
    ports: SpinLock<PortTable, BottomHalfDisabled>,
    sockets: SpinLock<SocketTable, BottomHalfDisabled>,
}

struct SocketTable {
    connections: BTreeMap<ConnId, Arc<ConnectionInner>>,
    listeners: BTreeMap<u32, Arc<ListenerInner>>,
}

impl VsockSpace {
    fn new(device: Arc<SocketDevice>) -> Self {
        Self {
            device,
            ports: SpinLock::new(PortTable::new()),
            sockets: SpinLock::new(SocketTable {
                connections: BTreeMap::new(),
                listeners: BTreeMap::new(),
            }),
        }
    }

    pub(super) fn device(&self) -> &SocketDevice {
        &self.device
    }

    pub(super) fn guest_cid(&self) -> u64 {
        self.device.guest_cid()
    }

    pub(super) fn lock_ports(&self) -> SpinLockGuard<'_, PortTable, BottomHalfDisabled> {
        self.ports.lock()
    }
}

impl VsockSpace {
    pub(super) fn new_connection(
        &self,
        bound_port: BoundPort,
        remote_addr: VsockSocketAddr,
        pollee: &Pollee,
    ) -> Result<Connection, (Error, BoundPort)> {
        use alloc::collections::btree_map::Entry;

        let mut sockets = self.sockets.lock();

        let conn_id = ConnId::from_port_and_remote(&bound_port, remote_addr);
        let Entry::Vacant(entry) = sockets.connections.entry(conn_id) else {
            return Err((
                errno!(EADDRINUSE, "the vsock connection already exists"),
                bound_port,
            ));
        };

        let inner = ConnectionInner::new_connecting(bound_port, &conn_id, pollee.clone());
        entry.insert(inner.clone());

        Ok(Connection::new(inner))
    }

    pub(super) fn remove_connection(&self, connection: &Arc<ConnectionInner>) {
        use alloc::collections::btree_map::Entry;

        let mut sockets = self.sockets.lock();

        let conn_id = connection.conn_id();

        let Entry::Occupied(occupied) = sockets.connections.entry(conn_id) else {
            return;
        };
        if !Arc::ptr_eq(connection, occupied.get()) {
            return;
        }

        occupied.remove();
    }

    pub(super) fn new_listener(
        &self,
        bound_port: BoundPort,
        backlog: usize,
        pollee: &Pollee,
    ) -> Result<Listener, (Error, BoundPort)> {
        use alloc::collections::btree_map::Entry;

        let mut sockets = self.sockets.lock();

        let port = bound_port.port();
        let Entry::Vacant(entry) = sockets.listeners.entry(port) else {
            return Err((
                errno!(EADDRINUSE, "the vsock listener already exists"),
                bound_port,
            ));
        };

        let inner = ListenerInner::new(bound_port, backlog, pollee.clone());
        entry.insert(inner.clone());

        Ok(Listener::new(inner))
    }

    pub(super) fn remove_listener(&self, listener: &Arc<ListenerInner>) {
        use alloc::collections::btree_map::Entry;

        let mut sockets = self.sockets.lock();

        let port = listener.bound_port().port();
        let removed = sockets.listeners.remove(&port);
        debug_assert!(
            removed
                .as_ref()
                .is_some_and(|removed| Arc::ptr_eq(removed, listener))
        );

        let connections = listener.take_incoming_on_removal();
        for connection in connections.into_iter() {
            let conn_id = connection.conn_id();

            let Entry::Occupied(occupied) = sockets.connections.entry(conn_id) else {
                continue;
            };
            if !Arc::ptr_eq(&connection, occupied.get()) {
                continue;
            }

            occupied.remove();

            connection.active_rst();
        }
    }
}

impl VsockSpace {
    pub(super) fn process_rx(&self) {
        let mut rx = self.device.lock_rx();
        let mut sockets = self.sockets.lock();

        while let Some(packet) = rx.recv() {
            self.process_rx_packet(&mut sockets, packet);
        }
    }

    fn process_rx_packet(&self, sockets: &mut SocketTable, packet: RxPacket) {
        use alloc::collections::btree_map::Entry;

        let header = packet.header();

        let conn_id = ConnId::from_incoming_header(&header);
        let entry = sockets.connections.entry(conn_id);

        match entry {
            Entry::Vacant(vacant) => {
                self.process_rx_with_listener(&sockets.listeners, vacant, &header, packet)
            }
            Entry::Occupied(occupied) => self.process_rx_with_connection(occupied, &header, packet),
        }
    }

    fn process_rx_with_listener(
        &self,
        listeners: &BTreeMap<u32, Arc<ListenerInner>>,
        vacant_conn: alloc::collections::btree_map::VacantEntry<'_, ConnId, Arc<ConnectionInner>>,
        header: &VirtioVsockHdr,
        packet: RxPacket,
    ) {
        if header.op() != Some(VirtioVsockOp::Request)
            || !self.validate_rx_header(VirtioVsockOp::Request, header, &packet)
        {
            self.send_raw_rst(header);
            return;
        }

        let dst_port = header.dst_port;
        let listener = if let Some(listener) = listeners.get(&dst_port)
            && !listener.is_full()
        {
            listener
        } else {
            self.send_raw_rst(header);
            return;
        };

        let bound_port = BoundPort::new_shared(listener.bound_port());
        let conn_id = vacant_conn.key();

        let inner = ConnectionInner::new_connected(bound_port, conn_id, header);
        vacant_conn.insert(inner.clone());

        listener.push_incoming(inner.clone());
    }

    fn process_rx_with_connection(
        &self,
        occupied_conn: alloc::collections::btree_map::OccupiedEntry<
            '_,
            ConnId,
            Arc<ConnectionInner>,
        >,
        header: &VirtioVsockHdr,
        packet: RxPacket,
    ) {
        let op = if let Some(op) = header.op()
            && self.validate_rx_header(op, header, &packet)
        {
            op
        } else {
            Self::reset_removed_connection(occupied_conn.remove());
            return;
        };

        let connection = occupied_conn.get();

        let should_remove = match op {
            VirtioVsockOp::Request => {
                connection.active_rst();
                true
            }
            VirtioVsockOp::Response => connection.on_response(header).is_err(),
            VirtioVsockOp::Rst => {
                connection.on_rst();
                true
            }
            VirtioVsockOp::Shutdown => connection.on_shutdown(header),
            VirtioVsockOp::Rw => connection.on_rw(header, packet).is_err(),
            VirtioVsockOp::CreditUpdate => connection.on_credit_update(header).is_err(),
            VirtioVsockOp::CreditRequest => connection.on_credit_request(header).is_err(),
        };

        if should_remove {
            Self::notify_removed_connection(occupied_conn.remove());
        }
    }

    fn send_raw_rst(&self, header: &VirtioVsockHdr) {
        if header.op == VirtioVsockOp::Rst as u16 {
            return;
        }

        let rst_header = VirtioVsockHdr {
            src_cid: header.dst_cid,
            dst_cid: header.src_cid,
            src_port: header.dst_port,
            dst_port: header.src_port,
            len: 0,
            type_: header.type_,
            op: VirtioVsockOp::Rst as u16,
            flags: 0,
            buf_alloc: 0,
            fwd_cnt: 0,
        };
        let _ = self.send_packet(&rst_header);
    }

    pub(super) fn process_transport_event(&self) {
        let mut sockets = self.sockets.lock();

        let connections = core::mem::take(&mut sockets.connections);
        for connection in connections.into_values() {
            connection.on_rst();
            Self::notify_removed_connection(connection);
        }

        self.device.reload_guest_id();
    }

    pub(super) fn process_timer_events(&self, events: Vec<TimerEvent>) {
        use alloc::collections::btree_map::Entry;

        let mut sockets = self.sockets.lock();

        for event in events.into_iter() {
            let Entry::Occupied(entry) = sockets.connections.entry(event.conn_id) else {
                continue;
            };

            if !entry.get().on_timeout(event.generation) {
                continue;
            }

            Self::notify_removed_connection(entry.remove());
        }
    }

    fn reset_removed_connection(connection: Arc<ConnectionInner>) {
        connection.active_rst();
        Self::notify_removed_connection(connection);
    }

    fn notify_removed_connection(connection: Arc<ConnectionInner>) {
        let pollee = connection.pollee().clone();
        drop(connection);

        pollee
            .notify(IoEvents::IN | IoEvents::OUT | IoEvents::RDHUP | IoEvents::HUP | IoEvents::ERR);
    }

    #[must_use]
    pub(super) fn send_packet(&self, header: &VirtioVsockHdr) -> bool {
        let Ok(builder) = TxPacket::new_builder() else {
            warn!("failed to allocate vsock packet: {:?}", header);
            return false;
        };
        let packet = builder.build(header);

        let mut tx = self.device.lock_tx();
        match tx.try_send(packet) {
            Ok(()) => (),
            Err(pending) => {
                pending.push_pending(None);
            }
        }

        true
    }

    fn validate_rx_header(
        &self,
        op: VirtioVsockOp,
        header: &VirtioVsockHdr,
        packet: &RxPacket,
    ) -> bool {
        if header.type_ != VirtioVsockType::Stream as u16 {
            return false;
        }

        if header.dst_cid != self.guest_cid() {
            return false;
        }

        let payload_len = packet.payload_len();
        if payload_len != header.len as usize {
            return false;
        }

        match op {
            VirtioVsockOp::Request
            | VirtioVsockOp::Response
            | VirtioVsockOp::Rst
            | VirtioVsockOp::CreditUpdate
            | VirtioVsockOp::CreditRequest => payload_len == 0 && header.flags == 0,
            VirtioVsockOp::Shutdown => {
                payload_len == 0 && VirtioVsockShutdownFlags::from_bits(header.flags).is_some()
            }
            VirtioVsockOp::Rw => header.flags == 0,
        }
    }
}

static VSOCK_SPACE: Once<VsockSpace> = Once::new();

pub(super) fn vsock_space() -> Result<&'static VsockSpace> {
    VSOCK_SPACE
        .get()
        .ok_or_else(|| errno!(ENODEV, "no virtio-vsock device is available"))
}

pub(super) fn init(device: Arc<SocketDevice>) {
    VSOCK_SPACE.call_once(move || VsockSpace::new(device));
}
