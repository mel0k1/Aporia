use bitflags::bitflags;
use ostd::mm::{Infallible, VmReader, VmWriter};

use super::util::read_payload;
use crate::{
    Attr, EntryReply, FuseError, FuseFileHandle, FuseOpcode, FuseOperation, FuseResult,
    ReplyExpectation,
};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct GetattrReq {
    getattr_flags: GetattrFlags,
    dummy: u32,
    fh: FuseFileHandle,
}

impl GetattrReq {
    pub const fn new(getattr_flags: GetattrFlags, fh: FuseFileHandle) -> Self {
        Self {
            getattr_flags,
            dummy: 0,
            fh,
        }
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct GetattrFlags(u32);

bitflags! {
    impl GetattrFlags: u32 {
        const GETATTR_FH = 1 << 0;

        const _          = !0;
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct FuseAttrReply {
    attr_valid: u64,
    attr_valid_nsec: u32,
    dummy: u32,
    attr: Attr,
}

impl FuseAttrReply {
    pub const fn new(attr_valid: u64, attr_valid_nsec: u32, attr: Attr) -> Self {
        Self {
            attr_valid,
            attr_valid_nsec,
            dummy: 0,
            attr,
        }
    }

    pub fn attr_valid(&self) -> u64 {
        self.attr_valid
    }

    pub fn attr_valid_nsec(&self) -> u32 {
        self.attr_valid_nsec
    }

    pub fn attr(&self) -> Attr {
        self.attr
    }
}

impl From<&EntryReply> for FuseAttrReply {
    fn from(entry_reply: &EntryReply) -> Self {
        Self::new(
            entry_reply.attr_valid(),
            entry_reply.attr_valid_nsec(),
            entry_reply.attr(),
        )
    }
}

pub struct GetattrOperation {
    getattr_req: GetattrReq,
}

impl GetattrOperation {
    pub fn new(getattr_req: GetattrReq) -> Self {
        Self { getattr_req }
    }
}

impl FuseOperation for GetattrOperation {
    type Output = FuseAttrReply;

    fn opcode(&self) -> FuseOpcode {
        FuseOpcode::Getattr
    }

    fn body_len(&self) -> usize {
        size_of::<GetattrReq>()
    }

    fn write_body(&mut self, writer: &mut VmWriter<'_, Infallible>) -> FuseResult<()> {
        writer
            .write_val(&self.getattr_req)
            .map_err(|_| FuseError::BufferTooSmall)
    }

    fn reply_expectation(&self) -> ReplyExpectation {
        ReplyExpectation::payload(size_of::<FuseAttrReply>())
    }

    fn parse_reply(
        payload_len: usize,
        reader: &mut VmReader<'_, Infallible>,
    ) -> FuseResult<Self::Output> {
        read_payload(payload_len, reader)
    }
}
