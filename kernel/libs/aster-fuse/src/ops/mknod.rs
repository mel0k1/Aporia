use ostd::mm::{Infallible, VmReader, VmWriter};

use super::util;
use crate::{EntryReply, FuseError, FuseOpcode, FuseOperation, FuseResult, ReplyExpectation};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct MknodReq {
    mode: u32,
    rdev: u32,
    umask: u32,
    padding: u32,
}

impl MknodReq {
    pub const fn new(mode: u32, rdev: u32) -> Self {
        Self {
            mode,
            rdev,
            umask: 0,
            padding: 0,
        }
    }
}

pub struct MknodOperation<'a> {
    mknod_req: MknodReq,
    name: &'a str,
}

impl<'a> MknodOperation<'a> {
    pub fn new(mknod_req: MknodReq, name: &'a str) -> Self {
        Self { mknod_req, name }
    }
}

impl FuseOperation for MknodOperation<'_> {
    type Output = EntryReply;

    fn opcode(&self) -> FuseOpcode {
        FuseOpcode::Mknod
    }

    fn body_len(&self) -> usize {
        util::name_body_len(size_of::<MknodReq>(), self.name)
    }

    fn write_body(&mut self, writer: &mut VmWriter<'_, Infallible>) -> FuseResult<()> {
        if writer.avail() < self.body_len() {
            return Err(FuseError::BufferTooSmall);
        }

        writer.write_val(&self.mknod_req).unwrap();
        writer.write(&mut VmReader::from(self.name.as_bytes()));
        writer.write(&mut VmReader::from(util::NAME_TERMINATOR));

        Ok(())
    }

    fn reply_expectation(&self) -> ReplyExpectation {
        ReplyExpectation::payload(size_of::<EntryReply>())
    }

    fn parse_reply(
        payload_len: usize,
        reader: &mut VmReader<'_, Infallible>,
    ) -> FuseResult<Self::Output> {
        util::read_payload(payload_len, reader)
    }
}
