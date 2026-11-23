use align_ext::AlignExt;

use super::{RecvFlags, SocketAddr};
use crate::{net::socket::unix::UnixControlMessage, prelude::*, util::net::CSocketOptionLevel};

#[derive(Debug)]
pub(crate) struct MessageHeader {
    pub(in crate::net) addr: Option<SocketAddr>,
    pub(in crate::net) control_messages: Vec<ControlMessage>,
}

impl MessageHeader {
    pub(crate) const fn new(
        addr: Option<SocketAddr>,
        control_messages: Vec<ControlMessage>,
    ) -> Self {
        Self {
            addr,
            control_messages,
        }
    }

    pub(crate) fn addr(&self) -> Option<&SocketAddr> {
        self.addr.as_ref()
    }

    pub(crate) fn control_messages(&self) -> &Vec<ControlMessage> {
        &self.control_messages
    }
}

#[derive(Debug)]
pub(crate) enum ControlMessage {
    Unix(UnixControlMessage),
}

impl ControlMessage {
    pub(crate) fn read_all_from(reader: &mut VmReader) -> Result<Vec<Self>> {
        const MAX_NR_MSGS: usize = 32;

        let mut msgs = Vec::new();

        while reader.has_remain() && msgs.len() < MAX_NR_MSGS {
            let header = reader.read_val::<CControlHeader>()?;
            if header.len < size_of::<CControlHeader>() || header.payload_len() > reader.remain() {
                return Err(errno!(EINVAL, "the size of the control message is invalid"));
            }

            if let Some(msg) = Self::read_from(&header, reader)? {
                msgs.push(msg);
            }

            let padding_len = header.padding_len().min(reader.remain());
            reader.skip(padding_len);
        }

        if reader.has_remain() {
            warn!("excessive control messages are currently not permitted");
            return Err(errno!(
                E2BIG,
                "excessive control messages are currently not permitted"
            ));
        }

        Ok(msgs)
    }

    fn read_from(header: &CControlHeader, reader: &mut VmReader) -> Result<Option<Self>> {
        let Some(level) = header.level() else {
            warn!("unsupported control message level in {:?}", header);
            reader.skip(header.payload_len());
            return Ok(None);
        };

        match level {
            CSocketOptionLevel::SOL_SOCKET => {
                let msg = UnixControlMessage::read_from(header, reader)?;
                Ok(msg.map(Self::Unix))
            }
            _ => {
                warn!("unsupported control message level in {:?}", header);
                reader.skip(header.payload_len());
                Ok(None)
            }
        }
    }

    pub(crate) fn write_all_to(msgs: &[Self], writer: &mut VmWriter) -> (usize, RecvFlags) {
        let mut len = 0;
        let mut output_flags = RecvFlags::empty();

        for msg in msgs.iter() {
            let (header, message_flags) = match msg.write_to(writer) {
                Ok(result) => result,
                Err(_) => {
                    output_flags |= RecvFlags::MSG_CTRUNC;
                    break;
                }
            };
            output_flags |= message_flags;

            len += header.total_len();

            let padding_len = header.padding_len().min(writer.avail());
            writer.skip(padding_len);
            len += padding_len;
        }

        (len, output_flags)
    }

    fn write_to(&self, writer: &mut VmWriter) -> Result<(CControlHeader, RecvFlags)> {
        match self {
            Self::Unix(msg) => msg.write_to(writer),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct CControlHeader {
    len: usize,
    level: i32,
    type_: i32,
}

const CMSG_ALIGN: usize = size_of::<usize>();

impl CControlHeader {
    pub(crate) fn new(level: CSocketOptionLevel, type_: i32, payload_len: usize) -> Self {
        Self {
            len: payload_len + size_of::<Self>(),
            level: level as i32,
            type_,
        }
    }

    pub(crate) fn payload_len_from_total(total_len: usize) -> Result<usize> {
        total_len
            .checked_sub(size_of::<Self>())
            .ok_or_else(|| errno!(EINVAL, "the control message buffer is too small"))
    }

    pub(crate) fn level(&self) -> Option<CSocketOptionLevel> {
        CSocketOptionLevel::try_from(self.level).ok()
    }

    pub(crate) fn type_(&self) -> i32 {
        self.type_
    }

    pub(crate) fn payload_len(&self) -> usize {
        self.len - size_of::<Self>()
    }

    pub(crate) fn total_len(&self) -> usize {
        self.len
    }

    pub(self) fn padding_len(&self) -> usize {
        self.total_len_with_padding() - self.total_len()
    }

    fn total_len_with_padding(&self) -> usize {
        self.len.align_up(CMSG_ALIGN)
    }
}
