use aster_bigtcp::wire::IpEndpoint;
use bound::BoundDatagram;
use unbound::{BindOptions, UnboundDatagram};

use super::{addr::UNSPECIFIED_LOCAL_ENDPOINT, ioctl::ipv4_ioctl};
use crate::{
    events::IoEvents,
    fs::file::FileCommon,
    net::{
        route::is_broadcast_endpoint,
        socket::{
            Socket,
            ip::{
                common::map_unspecified_to_localhost,
                options::{IpOptionSet, SetIpLevelOption},
            },
            new_socket_common,
            options::{Error as SocketError, SocketOption, macros::sock_option_mut},
            private::SocketPrivate,
            util::{
                MessageHeader, RecvFlags, RecvOutput, SendFlags, SocketAddr,
                datagram_common::{Bound, Inner, select_remote_and_bind},
                options::{
                    GetSocketLevelOption, SetSocketLevelOption, SocketOptionSet, SocketTimeouts,
                },
            },
        },
    },
    prelude::*,
    process::signal::{PollHandle, Pollable, Pollee},
    util::{MultiRead, MultiWrite, ioctl::RawIoctl, net::SockType},
};

mod bound;
pub(super) mod observer;
mod unbound;

pub(crate) struct DatagramSocket {
    inner: RwMutex<Inner<UnboundDatagram, BoundDatagram>>,
    options: RwLock<OptionSet>,
    timeouts: SocketTimeouts,

    pollee: Pollee,
    common: FileCommon,
}

#[derive(Clone, Debug)]
struct OptionSet {
    socket: SocketOptionSet,
    ip: IpOptionSet,
}

impl OptionSet {
    fn new() -> Self {
        let socket = SocketOptionSet::new_udp();
        let ip = IpOptionSet::new_udp();
        OptionSet { socket, ip }
    }
}

impl DatagramSocket {
    pub(crate) fn new(is_nonblocking: bool) -> Arc<Self> {
        let unbound_datagram = UnboundDatagram::new();
        Arc::new(Self {
            inner: RwMutex::new(Inner::Unbound(unbound_datagram)),
            options: RwLock::new(OptionSet::new()),
            timeouts: SocketTimeouts::new(),
            pollee: Pollee::new(),
            common: new_socket_common(is_nonblocking),
        })
    }

    fn try_recv(
        &self,
        writer: &mut dyn MultiWrite,
        flags: RecvFlags,
    ) -> Result<(RecvOutput, SocketAddr)> {
        let result = self
            .inner
            .read()
            .try_recv(writer, flags)
            .map(|(output, remote_endpoint)| (output, remote_endpoint.into()))?;
        self.pollee.invalidate();

        Ok(result)
    }

    fn try_send(
        &self,
        reader: &mut dyn MultiRead,
        remote: Option<&IpEndpoint>,
        flags: SendFlags,
    ) -> Result<usize> {
        let (sent_bytes, iface_to_poll) = select_remote_and_bind(
            &self.inner,
            remote,
            || {
                let remote_endpoint = remote.ok_or_else(|| {
                    errno!(EDESTADDRREQ, "the destination address is not specified",)
                })?;
                self.inner
                    .write()
                    .bind_ephemeral(remote_endpoint, &self.pollee)
            },
            |bound_datagram, remote_endpoint| {
                let sent_bytes = bound_datagram.try_send(reader, remote_endpoint, flags, ())?;
                let iface_to_poll = bound_datagram.iface().clone();
                Ok((sent_bytes, iface_to_poll))
            },
        )?;

        self.pollee.invalidate();
        iface_to_poll.poll();

        Ok(sent_bytes)
    }
}

impl Pollable for DatagramSocket {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee
            .poll_with(mask, poller, || self.inner.read().check_io_events())
    }
}

impl SocketPrivate for DatagramSocket {
    fn is_nonblocking(&self) -> bool {
        self.common.is_nonblocking()
    }

    fn protocol_ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        ipv4_ioctl(raw_ioctl)
    }
}

impl Socket for DatagramSocket {
    fn bind(&self, socket_addr: SocketAddr) -> Result<()> {
        let endpoint = socket_addr.try_into()?;
        let can_reuse = self.options.read().socket.reuse_addr();

        self.inner
            .write()
            .bind(&endpoint, &self.pollee, BindOptions { can_reuse })
    }

    fn connect(&self, socket_addr: SocketAddr) -> Result<()> {
        let endpoint = map_unspecified_to_localhost(socket_addr.try_into()?);
        let can_broadcast = self.options.read().socket.broadcast();
        if !can_broadcast && is_broadcast_endpoint(&endpoint) {
            return Err(errno!(
                EACCES,
                "connecting to a broadcast address without SO_BROADCAST is not allowed"
            ));
        }

        self.inner.write().connect(&endpoint, &self.pollee)
    }

    fn addr(&self) -> Result<SocketAddr> {
        let endpoint = self
            .inner
            .read()
            .addr()
            .unwrap_or(UNSPECIFIED_LOCAL_ENDPOINT);

        Ok(endpoint.into())
    }

    fn peer_addr(&self) -> Result<SocketAddr> {
        let endpoint = *self
            .inner
            .read()
            .peer_addr()
            .ok_or_else(|| errno!(ENOTCONN, "the socket is not connected"))?;

        Ok(endpoint.into())
    }

    fn sendmsg(
        &self,
        reader: &mut dyn MultiRead,
        message_header: MessageHeader,
        flags: SendFlags,
    ) -> Result<usize> {
        if !flags.is_all_supported() {
            warn!("unsupported flags: {:?}", flags);
        }

        let MessageHeader {
            addr,
            control_messages,
        } = message_header;

        let endpoint = match addr {
            Some(addr) => Some(addr.try_into()?),
            None => None,
        };

        if let Some(endpoint) = endpoint.as_ref() {
            let can_broadcast = self.options.read().socket.broadcast();
            if !can_broadcast && is_broadcast_endpoint(endpoint) {
                return Err(errno!(
                    EACCES,
                    "sending to a broadcast address without SO_BROADCAST is not allowed"
                ));
            }
        }

        if !control_messages.is_empty() {
            warn!("sending control message is not supported");
        }

        self.try_send(reader, endpoint.as_ref(), flags)
    }

    fn recvmsg(
        &self,
        writer: &mut dyn MultiWrite,
        flags: RecvFlags,
    ) -> Result<(RecvOutput, MessageHeader)> {
        if !flags.is_all_supported() {
            warn!("unsupported flags: {:?}", flags);
        }

        let (output, peer_addr) =
            self.block_on(IoEvents::IN, self.timeouts.recv_timeout(), || {
                self.try_recv(writer, flags)
            })?;

        let message_header = MessageHeader::new(Some(peer_addr), Vec::new());

        Ok((output, message_header))
    }

    fn get_option(&self, option: &mut dyn SocketOption) -> Result<()> {
        sock_option_mut!(match option {
            socket_errors @ SocketError => {
                socket_errors.set(None);
                return Ok(());
            }
            _ => (),
        });

        let inner = self.inner.read();
        let options = self.options.read();

        match options
            .socket
            .get_option(option, &(&*inner, &self.timeouts))
        {
            Err(err) if err.error() == Errno::ENOPROTOOPT => (),
            res => return res,
        }

        options.ip.get_option(option)
    }

    fn set_option(&self, option: &dyn SocketOption) -> Result<()> {
        let inner = self.inner.read();
        let mut options = self.options.write();

        let need_iface_poll = match options
            .socket
            .set_option(option, &(&*inner, &self.timeouts))
        {
            Err(err) if err.error() == Errno::ENOPROTOOPT => {
                options.ip.set_option(option, &*inner)?
            }
            Err(err) => return Err(err),
            Ok(need_iface_poll) => need_iface_poll,
        };

        let iface_to_poll = need_iface_poll
            .then(|| match &*inner {
                Inner::Unbound(_) => None,
                Inner::Bound(bound_datagram) => Some(bound_datagram.iface().clone()),
            })
            .flatten();

        drop(inner);
        drop(options);

        if let Some(iface) = iface_to_poll {
            iface.poll();
        }

        Ok(())
    }

    fn common(&self) -> &FileCommon {
        &self.common
    }
}

impl GetSocketLevelOption for (&Inner<UnboundDatagram, BoundDatagram>, &SocketTimeouts) {
    fn socket_type(&self) -> SockType {
        SockType::SOCK_DGRAM
    }

    fn is_listening(&self) -> bool {
        false
    }

    fn socket_timeouts(&self) -> Option<&SocketTimeouts> {
        Some(self.1)
    }
}

impl SetSocketLevelOption for (&Inner<UnboundDatagram, BoundDatagram>, &SocketTimeouts) {
    fn set_reuse_addr(&self, reuse_addr: bool) {
        let Inner::Bound(bound) = self.0 else {
            return;
        };

        bound.bound_port().set_can_reuse(reuse_addr);
    }

    fn socket_timeouts(&self) -> Option<&SocketTimeouts> {
        Some(self.1)
    }
}

impl SetIpLevelOption for Inner<UnboundDatagram, BoundDatagram> {
    fn set_hdrincl(&self, _hdrincl: bool) -> Result<()> {
        Err(errno!(
            ENOPROTOOPT,
            "IP_HDRINCL cannot be set on UDP sockets"
        ))
    }
}
