use super::{RecvFlags, RecvOutput, SendFlags};
use crate::{
    events::IoEvents,
    prelude::*,
    process::signal::Pollee,
    util::{MultiRead, MultiWrite},
};

pub(crate) trait Unbound {
    type Endpoint;
    type BindOptions;

    type Bound;

    fn bind(
        &mut self,
        endpoint: &Self::Endpoint,
        pollee: &Pollee,
        options: Self::BindOptions,
    ) -> Result<Self::Bound>;
    fn bind_ephemeral(
        &mut self,
        remote_endpoint: &Self::Endpoint,
        pollee: &Pollee,
    ) -> Result<Self::Bound>;

    fn check_io_events(&self) -> IoEvents;
}

pub(crate) trait Bound {
    type Endpoint;
    type SendOptions;

    fn local_endpoint(&self) -> Self::Endpoint;
    fn bind(&mut self, _endpoint: &Self::Endpoint) -> Result<()> {
        Err(errno!(EINVAL, "the socket is already bound to an address"))
    }
    fn remote_endpoint(&self) -> Option<&Self::Endpoint>;
    fn set_remote_endpoint(&mut self, endpoint: &Self::Endpoint);

    fn try_recv(
        &self,
        writer: &mut dyn MultiWrite,
        flags: RecvFlags,
    ) -> Result<(RecvOutput, Self::Endpoint)>;
    fn try_send(
        &self,
        reader: &mut dyn MultiRead,
        remote: &Self::Endpoint,
        flags: SendFlags,
        options: Self::SendOptions,
    ) -> Result<usize>;

    fn check_io_events(&self) -> IoEvents;
}

pub(crate) enum Inner<UnboundSocket, BoundSocket> {
    Unbound(UnboundSocket),
    Bound(BoundSocket),
}

impl<UnboundSocket, BoundSocket> Inner<UnboundSocket, BoundSocket>
where
    UnboundSocket: Unbound<Endpoint = BoundSocket::Endpoint, Bound = BoundSocket>,
    BoundSocket: Bound,
{
    pub(crate) fn bind(
        &mut self,
        endpoint: &UnboundSocket::Endpoint,
        pollee: &Pollee,
        options: UnboundSocket::BindOptions,
    ) -> Result<()> {
        let unbound_datagram = match self {
            Inner::Unbound(unbound_datagram) => unbound_datagram,
            Inner::Bound(bound_datagram) => {
                return bound_datagram.bind(endpoint);
            }
        };

        let bound_datagram = unbound_datagram.bind(endpoint, pollee, options)?;
        *self = Inner::Bound(bound_datagram);

        Ok(())
    }

    pub(crate) fn bind_ephemeral(
        &mut self,
        remote_endpoint: &UnboundSocket::Endpoint,
        pollee: &Pollee,
    ) -> Result<()> {
        let unbound_datagram = match self {
            Inner::Unbound(unbound_datagram) => unbound_datagram,
            Inner::Bound(_) => return Ok(()),
        };

        let bound_datagram = unbound_datagram.bind_ephemeral(remote_endpoint, pollee)?;
        *self = Inner::Bound(bound_datagram);

        Ok(())
    }

    pub(crate) fn connect(
        &mut self,
        remote_endpoint: &UnboundSocket::Endpoint,
        pollee: &Pollee,
    ) -> Result<()> {
        self.bind_ephemeral(remote_endpoint, pollee)?;

        let bound_datagram = match self {
            Inner::Unbound(_) => {
                unreachable!(
                    "`bind_to_ephemeral_endpoint` succeeds so the socket cannot be unbound"
                );
            }
            Inner::Bound(bound_datagram) => bound_datagram,
        };
        bound_datagram.set_remote_endpoint(remote_endpoint);

        Ok(())
    }

    pub(crate) fn addr(&self) -> Option<UnboundSocket::Endpoint> {
        match self {
            Inner::Unbound(_) => None,
            Inner::Bound(bound_datagram) => Some(bound_datagram.local_endpoint()),
        }
    }

    pub(crate) fn peer_addr(&self) -> Option<&UnboundSocket::Endpoint> {
        match self {
            Inner::Unbound(_) => None,
            Inner::Bound(bound_datagram) => bound_datagram.remote_endpoint(),
        }
    }

    pub(crate) fn try_recv(
        &self,
        writer: &mut dyn MultiWrite,
        flags: RecvFlags,
    ) -> Result<(RecvOutput, UnboundSocket::Endpoint)> {
        match self {
            Inner::Unbound(_) => Err(errno!(EAGAIN, "the socket is not bound")),
            Inner::Bound(bound_datagram) => bound_datagram.try_recv(writer, flags),
        }
    }

    pub(crate) fn check_io_events(&self) -> IoEvents {
        match self {
            Inner::Unbound(unbound_datagram) => unbound_datagram.check_io_events(),
            Inner::Bound(bound_datagram) => bound_datagram.check_io_events(),
        }
    }
}

pub(crate) fn select_remote_and_bind<UnboundSocket, BoundSocket, B, F, R>(
    inner_mutex: &RwMutex<Inner<UnboundSocket, BoundSocket>>,
    remote: Option<&UnboundSocket::Endpoint>,
    bind_ephemeral: B,
    op: F,
) -> Result<R>
where
    UnboundSocket: Unbound<Endpoint = BoundSocket::Endpoint, Bound = BoundSocket>,
    BoundSocket: Bound,
    B: FnOnce() -> Result<()>,
    F: FnOnce(&BoundSocket, &UnboundSocket::Endpoint) -> Result<R>,
{
    let mut inner = inner_mutex.read();

    #[expect(clippy::never_loop)]
    let bound_datagram = loop {
        if let Inner::Bound(bound_datagram) = &*inner {
            break bound_datagram;
        }

        drop(inner);
        bind_ephemeral()?;
        inner = inner_mutex.read();

        if let Inner::Bound(bound_datagram) = &*inner {
            break bound_datagram;
        }
        unreachable!("`try_bind_ephemeral` succeeds so the socket cannot be unbound");
    };

    let remote_endpoint = remote
        .or_else(|| bound_datagram.remote_endpoint())
        .ok_or_else(|| errno!(EDESTADDRREQ, "the destination address is not specified",))?;

    op(bound_datagram, remote_endpoint)
}
