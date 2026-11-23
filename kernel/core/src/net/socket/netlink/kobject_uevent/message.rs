use crate::{
    net::socket::netlink::{
        NetlinkSocketAddr, receiver::QueueableMessage, table::MulticastMessage,
    },
    prelude::*,
    util::MultiWrite,
};

#[derive(Clone, Debug)]
pub(crate) struct UeventMessage {
    raw: Arc<[u8]>,
    src_addr: NetlinkSocketAddr,
}

impl UeventMessage {
    pub(super) fn new(payload: Vec<u8>, src_addr: NetlinkSocketAddr) -> Self {
        Self {
            raw: payload.into(),
            src_addr,
        }
    }

    pub(super) fn src_addr(&self) -> &NetlinkSocketAddr {
        &self.src_addr
    }

    pub(super) fn write_to(&self, writer: &mut dyn MultiWrite) -> Result<()> {
        let _nbytes = writer.write(&mut VmReader::from(self.raw.as_ref()))?;

        Ok(())
    }
}

impl QueueableMessage for UeventMessage {
    fn total_len(&self) -> usize {
        self.raw.len()
    }
}

impl MulticastMessage for UeventMessage {}
