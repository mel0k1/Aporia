use smoltcp::wire::{IpAddress, IpEndpoint};

pub struct BindPortConfig {
    addr: IpAddress,
    kind: PortKind,
}

enum PortKind {
    CanReuse(u16),
    Specified(u16),
    Ephemeral(bool),
    Backlog(u16),
}

impl BindPortConfig {
    pub fn new(endpoint: IpEndpoint, can_reuse: bool) -> Self {
        let port = endpoint.port;
        let kind = match (port, can_reuse) {
            (0, can_reuse) => PortKind::Ephemeral(can_reuse),
            (_, true) => PortKind::CanReuse(port),
            (_, false) => PortKind::Specified(port),
        };
        Self {
            addr: endpoint.addr,
            kind,
        }
    }

    pub fn new_backlog(endpoint: IpEndpoint) -> Self {
        Self {
            addr: endpoint.addr,
            kind: PortKind::Backlog(endpoint.port),
        }
    }

    pub(super) fn is_backlog(&self) -> bool {
        matches!(self.kind, PortKind::Backlog(..))
    }

    pub(super) fn can_reuse(&self) -> bool {
        matches!(
            self.kind,
            PortKind::CanReuse(..) | PortKind::Ephemeral(true)
        )
    }

    pub(super) fn port(&self) -> Option<u16> {
        match &self.kind {
            PortKind::CanReuse(port) | PortKind::Specified(port) | PortKind::Backlog(port) => {
                Some(*port)
            }
            PortKind::Ephemeral(_) => None,
        }
    }

    pub(super) fn addr(&self) -> IpAddress {
        self.addr
    }
}
