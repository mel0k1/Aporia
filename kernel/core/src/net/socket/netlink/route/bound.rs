use super::message::{RtnlMessage, RtnlSegment};
use crate::{
    events::IoEvents,
    net::socket::{
        netlink::{
            NetlinkSocketAddr,
            common::BoundNetlink,
            message::{ContinueRead, ProtocolSegment},
            options::NetlinkOptionSet,
            route::kernel::get_netlink_route_kernel,
        },
        util::{RecvFlags, RecvOutput, SendFlags, datagram_common},
    },
    prelude::*,
    util::{MultiRead, MultiWrite},
};

pub(super) type BoundNetlinkRoute = BoundNetlink<RtnlMessage>;

impl datagram_common::Bound for BoundNetlinkRoute {
    type Endpoint = NetlinkSocketAddr;
    type SendOptions = NetlinkOptionSet;

    fn local_endpoint(&self) -> Self::Endpoint {
        self.handle.addr()
    }

    fn bind(&mut self, endpoint: &Self::Endpoint) -> Result<()> {
        self.bind_common(endpoint)
    }

    fn remote_endpoint(&self) -> Option<&Self::Endpoint> {
        Some(&self.remote_addr)
    }

    fn set_remote_endpoint(&mut self, endpoint: &Self::Endpoint) {
        self.remote_addr = *endpoint;
    }

    fn try_send(
        &self,
        reader: &mut dyn MultiRead,
        remote: &Self::Endpoint,
        flags: SendFlags,
        options: Self::SendOptions,
    ) -> Result<usize> {
        if !flags.is_all_supported() {
            warn!("unsupported flags: {:?}", flags);
        }

        if *remote != NetlinkSocketAddr::new_unspecified() {
            return Err(errno!(
                ECONNREFUSED,
                "sending netlink route messages to user space is not supported"
            ));
        }

        let sum_lens = reader.sum_lens();

        let local_port = self.handle.port();
        let rtnl_kernel = get_netlink_route_kernel();
        let strict_check = options.strict_check();

        loop {
            let mut segment = match RtnlSegment::read_from(reader, strict_check) {
                Ok(ContinueRead::Parsed(seg)) => seg,

                Ok(ContinueRead::SkippedInvalid(err_segment)) => {
                    rtnl_kernel.report_error(err_segment, local_port);
                    continue;
                }

                Err(err) if err.error() == Errno::EFAULT => {
                    return Err(err);
                }

                Err(_) => break,
            };

            let header = segment.header_mut();
            if header.pid == 0 {
                header.pid = local_port;
            }

            rtnl_kernel.handle_request(&segment, local_port, strict_check);
        }

        Ok(sum_lens)
    }

    fn try_recv(
        &self,
        writer: &mut dyn MultiWrite,
        flags: RecvFlags,
    ) -> Result<(RecvOutput, NetlinkSocketAddr)> {
        if !flags.is_all_supported() {
            warn!("unsupported flags: {:?}", flags);
        }

        let mut receive_queue = self.receive_queue.lock();

        receive_queue.dequeue_if(|response, response_len| {
            let copied_len = response_len.min(writer.sum_lens());
            response.write_to(writer)?;

            let remote = NetlinkSocketAddr::new_unspecified();

            let should_dequeue = flags.receive_behavior().will_consume_data();
            let output = RecvOutput::new_for_packet(flags, copied_len, response_len);
            Ok((should_dequeue, (output, remote)))
        })
    }

    fn check_io_events(&self) -> IoEvents {
        self.check_io_events_common()
    }
}
