use bitflags::bitflags;
use ostd::mm::{Infallible, VmReader, VmWriter};

use super::util::read_payload;
use crate::{FuseError, FuseFileHandle, FuseOpcode, FuseOperation, FuseResult, ReplyExpectation};

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct WriteFlags(u32);

bitflags! {
    impl WriteFlags: u32 {
        const WRITE_CACHE     = 1 << 0;
        const WRITE_LOCKOWNER = 1 << 1;

        const _               = !0;
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct WriteReq {
    fh: FuseFileHandle,
    offset: u64,
    size: u32,
    write_flags: WriteFlags,
    lock_owner: u64,
    flags: u32,
    padding: u32,
}

impl WriteReq {
    pub const fn new(
        fh: FuseFileHandle,
        offset: u64,
        size: u32,
        flags: u32,
        write_flags: WriteFlags,
    ) -> Self {
        Self {
            fh,
            offset,
            size,
            write_flags,
            lock_owner: 0,
            flags,
            padding: 0,
        }
    }

    pub fn size(&self) -> u32 {
        self.size
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct WriteReply {
    size: u32,
    padding: u32,
}

impl WriteReply {
    pub fn size(&self) -> usize {
        self.size as usize
    }
}

pub struct WriteOperation {
    write_req: WriteReq,
}

impl WriteOperation {
    pub fn new(write_req: WriteReq) -> Self {
        Self { write_req }
    }

    pub fn payload_size(&self) -> usize {
        self.write_req.size() as usize
    }
}

impl FuseOperation for WriteOperation {
    type Output = WriteReply;

    fn opcode(&self) -> FuseOpcode {
        FuseOpcode::Write
    }

    fn body_len(&self) -> usize {
        size_of::<WriteReq>()
    }

    fn write_body(&mut self, writer: &mut VmWriter<'_, Infallible>) -> FuseResult<()> {
        writer
            .write_val(&self.write_req)
            .map_err(|_| FuseError::BufferTooSmall)
    }

    fn reply_expectation(&self) -> ReplyExpectation {
        ReplyExpectation::WritePayload {
            requested_size: self.write_req.size() as usize,
        }
    }

    fn parse_reply(
        payload_len: usize,
        reader: &mut VmReader<'_, Infallible>,
    ) -> FuseResult<Self::Output> {
        read_payload(payload_len, reader)
    }
}
