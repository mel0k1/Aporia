use crate::{
    net::socket::netlink::{addr::PortNum, receiver::QueueableMessage},
    prelude::*,
};

pub(super) struct MulticastGroup {
    members: BTreeSet<PortNum>,
}

impl MulticastGroup {
    pub(super) const fn new() -> Self {
        Self {
            members: BTreeSet::new(),
        }
    }

    pub(super) fn add_member(&mut self, port_num: PortNum) {
        self.members.insert(port_num);
    }

    pub(super) fn remove_member(&mut self, port_num: PortNum) {
        self.members.remove(&port_num);
    }

    pub(super) fn members(&self) -> &BTreeSet<PortNum> {
        &self.members
    }
}

pub(crate) trait MulticastMessage: QueueableMessage + Clone {}
