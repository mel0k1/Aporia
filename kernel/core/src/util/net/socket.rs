use super::read_socket_addr_from_user;
use crate::{
    net::socket::util::{ControlMessage, RecvFlags, SocketAddr},
    prelude::*,
    util::{
        VmReaderArray, VmWriterArray, iovec::MAX_IO_VECTOR_LENGTH,
        net::write_socket_addr_with_max_len,
    },
};

/// Standard well-defined IP protocols.
/// From <https://elixir.bootlin.com/linux/v6.0.9/source/include/uapi/linux/in.h>.
#[expect(non_camel_case_types)]
#[repr(i32)]
#[derive(Clone, Copy, Debug, TryFromInt)]
pub(crate) enum Protocol {
    IPPROTO_IP = 0,
    IPPROTO_ICMP = 1,
    IPPROTO_IGMP = 2,
    IPPROTO_TCP = 6,
    IPPROTO_EGP = 8,
    IPPROTO_PUP = 12,
    IPPROTO_UDP = 17,
    IPPROTO_IDP = 22,
    IPPROTO_TP = 29,
    IPPROTO_DCCP = 33,
    IPPROTO_IPV6 = 41,
    IPPROTO_RSVP = 46,
    IPPROTO_GRE = 47,
    IPPROTO_ESP = 50,
    IPPROTO_AH = 51,
    IPPROTO_MTP = 92,
    IPPROTO_BEETPH = 94,
    IPPROTO_ENCAP = 98,
    IPPROTO_PIM = 103,
    IPPROTO_COMP = 108,
    IPPROTO_SCTP = 132,
    IPPROTO_UDPLITE = 136,
    IPPROTO_MPLS = 137,
    IPPROTO_ETHERNET = 143,
    IPPROTO_RAW = 255,
    IPPROTO_MPTCP = 262,
}

/// Socket types.
/// From <https://elixir.bootlin.com/linux/v6.0.9/source/include/linux/net.h>.
#[expect(non_camel_case_types)]
#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub(crate) enum SockType {
    /// Stream socket
    SOCK_STREAM = 1,
    /// Datagram socket
    SOCK_DGRAM = 2,
    /// Raw socket
    SOCK_RAW = 3,
    /// Reliably-delivered message
    SOCK_RDM = 4,
    /// Sequential packet socket
    SOCK_SEQPACKET = 5,
    /// Datagram Congestion Control Protocol socket
    SOCK_DCCP = 6,
    /// Linux specific way of getting packets at the dev level
    SOCK_PACKET = 10,
}

pub(crate) const SOCK_TYPE_MASK: i32 = 0xf;

bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub(crate) struct SockFlags: i32 {
        const SOCK_NONBLOCK = 1 << 11;
        const SOCK_CLOEXEC  = 1 << 19;
    }
}

#[padding_struct]
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct CUserMsgHdr {
    /// Pointer to socket address structure
    pub msg_name: Vaddr,
    /// Size of socket address
    pub msg_namelen: i32,
    /// Scatter/Gather iov array
    pub msg_iov: Vaddr,
    /// The # of elements in msg_iov
    pub msg_iovlen: usize,
    /// Ancillary data
    pub msg_control: Vaddr,
    /// Ancillary data buffer length
    pub msg_controllen: usize,
    /// Flags on received message
    pub msg_flags: u32,
}

impl CUserMsgHdr {
    pub(crate) fn read_socket_addr_from_user(&self) -> Result<Option<SocketAddr>> {
        if self.msg_name == 0 {
            return Ok(None);
        }

        let socket_addr = read_socket_addr_from_user(self.msg_name, self.msg_namelen as usize)?;
        Ok(Some(socket_addr))
    }

    pub(crate) fn write_socket_addr_to_user(&self, addr: Option<&SocketAddr>) -> Result<i32> {
        if self.msg_name == 0 {
            return Ok(self.msg_namelen);
        }

        let actual_len = if let Some(addr) = addr {
            write_socket_addr_with_max_len(addr, self.msg_name, self.msg_namelen)?
        } else {
            0
        };
        Ok(actual_len)
    }

    pub(crate) fn read_control_messages_from_user(
        &self,
        user_space: &CurrentUserSpace,
    ) -> Result<Vec<ControlMessage>> {
        if self.msg_control == 0 {
            return Ok(Vec::new());
        }

        let mut reader = user_space.reader(self.msg_control, self.msg_controllen)?;
        let control_messages = ControlMessage::read_all_from(&mut reader)?;
        Ok(control_messages)
    }

    pub(crate) fn write_control_messages_to_user(
        &self,
        control_messages: &[ControlMessage],
        user_space: &CurrentUserSpace,
    ) -> Result<(u32, RecvFlags)> {
        if self.msg_control == 0 {
            let output_flags = if control_messages.is_empty() {
                RecvFlags::empty()
            } else {
                RecvFlags::MSG_CTRUNC
            };
            return Ok((0, output_flags));
        }

        let mut writer = user_space.writer(self.msg_control, self.msg_controllen)?;
        let (write_len, output_flags) = ControlMessage::write_all_to(control_messages, &mut writer);
        Ok((write_len as u32, output_flags))
    }

    pub(crate) fn copy_reader_array_from_user<'a>(
        &self,
        user_space: &'a CurrentUserSpace<'a>,
    ) -> Result<VmReaderArray<'a>> {
        if self.msg_iovlen > MAX_IO_VECTOR_LENGTH {
            return Err(errno!(EMSGSIZE, "the I/O vector contains too many buffers"));
        }

        VmReaderArray::from_user_io_vecs(user_space, self.msg_iov, self.msg_iovlen)
    }

    pub(crate) fn copy_writer_array_from_user<'a>(
        &self,
        user_space: &'a CurrentUserSpace<'a>,
    ) -> Result<VmWriterArray<'a>> {
        if self.msg_iovlen > MAX_IO_VECTOR_LENGTH {
            return Err(errno!(EMSGSIZE, "the I/O vector contains too many buffers"));
        }

        VmWriterArray::from_user_io_vecs(user_space, self.msg_iov, self.msg_iovlen)
    }
}
