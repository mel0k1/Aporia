use multicast::MulticastGroup;
pub(super) use multicast::MulticastMessage;
use spin::Once;

use super::{
    addr::{GroupIdSet, MAX_GROUPS, NetlinkProtocolId, NetlinkSocketAddr, PortNum},
    receiver::QueueableMessage,
};
use crate::{
    net::socket::netlink::{
        addr::UNSPECIFIED_PORT, kobject_uevent::UeventMessage, receiver::MessageReceiver,
        route::RtnlMessage,
    },
    prelude::*,
    util::random::getrandom,
};

mod multicast;

static NETLINK_SOCKET_TABLE: Once<NetlinkSocketTable> = Once::new();

struct NetlinkSocketTable {
    route: RwMutex<ProtocolSocketTable<RtnlMessage>>,
    uevent: RwMutex<ProtocolSocketTable<UeventMessage>>,
}

impl NetlinkSocketTable {
    fn new() -> Self {
        Self {
            route: RwMutex::new(ProtocolSocketTable::new()),
            uevent: RwMutex::new(ProtocolSocketTable::new()),
        }
    }
}

pub(crate) trait SupportedNetlinkProtocol {
    type Message: 'static + Send;

    fn socket_table() -> &'static RwMutex<ProtocolSocketTable<Self::Message>>;

    fn bind(
        addr: &NetlinkSocketAddr,
        receiver: MessageReceiver<Self::Message>,
    ) -> Result<BoundHandle<Self::Message>> {
        let mut socket_table = Self::socket_table().write();
        socket_table.bind(Self::socket_table(), addr, receiver)
    }

    fn unicast(dst_port: PortNum, message: Self::Message) -> Result<()>
    where
        Self::Message: QueueableMessage,
    {
        let socket_table = Self::socket_table().read();
        socket_table.unicast(dst_port, message)
    }

    fn multicast(dst_groups: GroupIdSet, message: Self::Message) -> Result<()>
    where
        Self::Message: MulticastMessage,
    {
        let socket_table = Self::socket_table().read();
        socket_table.multicast(dst_groups, message)
    }
}

pub(crate) enum NetlinkRouteProtocol {}

impl SupportedNetlinkProtocol for NetlinkRouteProtocol {
    type Message = RtnlMessage;

    fn socket_table() -> &'static RwMutex<ProtocolSocketTable<Self::Message>> {
        &NETLINK_SOCKET_TABLE.get().unwrap().route
    }
}

pub(crate) enum NetlinkUeventProtocol {}

impl SupportedNetlinkProtocol for NetlinkUeventProtocol {
    type Message = UeventMessage;

    fn socket_table() -> &'static RwMutex<ProtocolSocketTable<Self::Message>> {
        &NETLINK_SOCKET_TABLE.get().unwrap().uevent
    }
}

pub(crate) struct ProtocolSocketTable<Message> {
    unicast_sockets: BTreeMap<PortNum, MessageReceiver<Message>>,
    multicast_groups: Box<[MulticastGroup]>,
}

impl<Message: 'static> ProtocolSocketTable<Message> {
    fn new() -> Self {
        let multicast_groups = (0u32..MAX_GROUPS).map(|_| MulticastGroup::new()).collect();
        Self {
            unicast_sockets: BTreeMap::new(),
            multicast_groups,
        }
    }

    fn bind(
        &mut self,
        socket_table: &'static RwMutex<ProtocolSocketTable<Message>>,
        addr: &NetlinkSocketAddr,
        receiver: MessageReceiver<Message>,
    ) -> Result<BoundHandle<Message>> {
        let port = if addr.port() != UNSPECIFIED_PORT {
            addr.port()
        } else {
            let mut random_port = current!().pid();
            while random_port == UNSPECIFIED_PORT || self.unicast_sockets.contains_key(&random_port)
            {
                getrandom(random_port.as_mut_bytes());
            }
            random_port
        };

        if self.unicast_sockets.contains_key(&port) {
            return Err(errno!(EADDRINUSE, "the netlink port is already in use"));
        }

        self.unicast_sockets.insert(port, receiver);

        for group_id in addr.groups().ids_iter() {
            let group = &mut self.multicast_groups[group_id as usize];
            group.add_member(port);
        }

        Ok(BoundHandle::new(socket_table, port, addr.groups()))
    }

    fn unicast(&self, dst_port: PortNum, message: Message) -> Result<()>
    where
        Message: QueueableMessage,
    {
        let Some(receiver) = self.unicast_sockets.get(&dst_port) else {
            return Ok(());
        };
        receiver.enqueue_message(message);

        Ok(())
    }

    fn multicast(&self, dst_groups: GroupIdSet, message: Message) -> Result<()>
    where
        Message: MulticastMessage,
    {
        for group in dst_groups.ids_iter() {
            let Some(group) = self.multicast_groups.get(group as usize) else {
                continue;
            };

            for port_num in group.members() {
                let Some(receiver) = self.unicast_sockets.get(port_num) else {
                    continue;
                };
                receiver.enqueue_message(message.clone());
            }
        }

        Ok(())
    }
}

pub(crate) struct BoundHandle<Message: 'static> {
    socket_table: &'static RwMutex<ProtocolSocketTable<Message>>,
    port: PortNum,
    groups: GroupIdSet,
}

impl<Message: 'static> BoundHandle<Message> {
    fn new(
        socket_table: &'static RwMutex<ProtocolSocketTable<Message>>,
        port: PortNum,
        groups: GroupIdSet,
    ) -> Self {
        debug_assert_ne!(port, UNSPECIFIED_PORT);

        Self {
            socket_table,
            port,
            groups,
        }
    }

    pub(super) const fn port(&self) -> PortNum {
        self.port
    }

    pub(super) const fn addr(&self) -> NetlinkSocketAddr {
        NetlinkSocketAddr::new(self.port, self.groups)
    }

    pub(super) fn add_groups(&mut self, groups: GroupIdSet) {
        let mut protocol_sockets = self.socket_table.write();

        for group_id in groups.ids_iter() {
            let group = &mut protocol_sockets.multicast_groups[group_id as usize];
            group.add_member(self.port);
        }

        self.groups.add_groups(groups);
    }

    pub(super) fn drop_groups(&mut self, groups: GroupIdSet) {
        let mut protocol_sockets = self.socket_table.write();

        for group_id in groups.ids_iter() {
            let group = &mut protocol_sockets.multicast_groups[group_id as usize];
            group.remove_member(self.port);
        }

        self.groups.drop_groups(groups);
    }

    pub(super) fn bind_groups(&mut self, groups: GroupIdSet) {
        let mut protocol_sockets = self.socket_table.write();

        for group_id in self.groups.ids_iter() {
            let group = &mut protocol_sockets.multicast_groups[group_id as usize];
            group.remove_member(self.port);
        }

        for group_id in groups.ids_iter() {
            let group = &mut protocol_sockets.multicast_groups[group_id as usize];
            group.add_member(self.port);
        }

        self.groups = groups;
    }
}

impl<Message: 'static> Drop for BoundHandle<Message> {
    fn drop(&mut self) {
        let mut protocol_sockets = self.socket_table.write();

        protocol_sockets.unicast_sockets.remove(&self.port);

        for group_id in self.groups.ids_iter() {
            let group = &mut protocol_sockets.multicast_groups[group_id as usize];
            group.remove_member(self.port);
        }
    }
}

pub(super) fn init() {
    NETLINK_SOCKET_TABLE.call_once(NetlinkSocketTable::new);
}

pub(crate) fn is_valid_protocol(protocol: NetlinkProtocolId) -> bool {
    protocol < MAX_ALLOWED_PROTOCOL_ID
}

#[expect(non_camel_case_types)]
#[expect(clippy::upper_case_acronyms)]
#[repr(u32)]
#[derive(Clone, Copy, Debug, TryFromInt)]
pub(crate) enum StandardNetlinkProtocol {
    ROUTE = 0,
    UNUSED = 1,
    USERSOCK = 2,
    FIREWALL = 3,
    SOCK_DIAG = 4,
    NFLOG = 5,
    XFRM = 6,
    SELINUX = 7,
    ISCSI = 8,
    AUDIT = 9,
    FIB_LOOKUP = 10,
    CONNECTOR = 11,
    NETFILTER = 12,
    IP6_FW = 13,
    DNRTMSG = 14,
    KOBJECT_UEVENT = 15,
    GENERIC = 16,
    SCSITRANSPORT = 18,
    ECRYPTFS = 19,
    RDMA = 20,
    CRYPTO = 21,
    SMC = 22,
}

const MAX_ALLOWED_PROTOCOL_ID: NetlinkProtocolId = 32;
