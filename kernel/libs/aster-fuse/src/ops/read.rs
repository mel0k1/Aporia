use bitflags::bitflags;
use ostd::mm::{Infallible, VmReader, VmWriter};

use crate::{FuseError, FuseFileHandle, FuseOpcode, FuseOperation, FuseResult, ReplyExpectation};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct ReadReq {
    fh: FuseFileHandle,
    offset: u64,
    size: u32,
    read_flags: ReadFlags,
    lock_owner: u64,
    flags: u32,
    padding: u32,
}

impl ReadReq {
    pub const fn new(fh: FuseFileHandle, offset: u64, size: u32, flags: u32) -> Self {
        Self {
            fh,
            offset,
            size,
            read_flags: ReadFlags::empty(),
            lock_owner: 0,
            flags,
            padding: 0,
        }
    }

    pub fn size(&self) -> u32 {
        self.size
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct ReadFlags(u32);

bitflags! {
    impl ReadFlags: u32 {
        const READ_LOCKOWNER = 1 << 1;

        const _              = !0;
    }
}

pub struct ReadOperation {
    read_req: ReadReq,
}

impl ReadOperation {
    pub fn new(read_req: ReadReq) -> Self {
        Self { read_req }
    }
}

impl FuseOperation for ReadOperation {
    type Output = usize;

    fn opcode(&self) -> FuseOpcode {
        FuseOpcode::Read
    }

    fn body_len(&self) -> usize {
        size_of::<ReadReq>()
    }

    fn write_body(&mut self, writer: &mut VmWriter<'_, Infallible>) -> FuseResult<()> {
        writer
            .write_val(&self.read_req)
            .map_err(|_| FuseError::BufferTooSmall)
    }

    fn reply_expectation(&self) -> ReplyExpectation {
        ReplyExpectation::variable_payload(self.read_req.size() as usize)
    }

    fn parse_reply(
        payload_len: usize,
        _reader: &mut VmReader<'_, Infallible>,
    ) -> FuseResult<Self::Output> {
        Ok(payload_len)
    }
}
