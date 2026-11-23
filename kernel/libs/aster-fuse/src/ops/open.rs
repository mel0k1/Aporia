use bitflags::bitflags;
use ostd::mm::{Infallible, VmReader, VmWriter};

use super::util::read_payload;
use crate::{FuseError, FuseFileHandle, FuseOpcode, FuseOperation, FuseResult, ReplyExpectation};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct OpenReq {
    flags: u32,
    open_flags: FuseOpenFlags,
}

impl OpenReq {
    pub const fn new(flags: u32) -> Self {
        Self {
            flags,
            open_flags: FuseOpenFlags::empty(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct OpenReply {
    fh: FuseFileHandle,
    open_flags: FuseOpenFlags,
    padding: u32,
}

impl OpenReply {
    pub fn fh(&self) -> FuseFileHandle {
        self.fh
    }

    pub fn open_flags(&self) -> FuseOpenFlags {
        self.open_flags
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct FuseOpenFlags(u32);

bitflags! {
    impl FuseOpenFlags: u32 {
        const FOPEN_DIRECT_IO              = 1 << 0;
        const FOPEN_KEEP_CACHE             = 1 << 1;
        const FOPEN_NONSEEKABLE            = 1 << 2;
        const FOPEN_CACHE_DIR              = 1 << 3;
        const FOPEN_STREAM                 = 1 << 4;
        const FOPEN_NOFLUSH                = 1 << 5;
        const FOPEN_PARALLEL_DIRECT_WRITES = 1 << 6;
        const FOPEN_PASSTHROUGH            = 1 << 7;

        const _                            = !0;
    }
}

pub struct OpenOperation {
    open_req: OpenReq,
}

impl OpenOperation {
    pub fn new(open_req: OpenReq) -> Self {
        Self { open_req }
    }
}

impl FuseOperation for OpenOperation {
    type Output = OpenReply;

    fn opcode(&self) -> FuseOpcode {
        FuseOpcode::Open
    }

    fn body_len(&self) -> usize {
        size_of::<OpenReq>()
    }

    fn write_body(&mut self, writer: &mut VmWriter<'_, Infallible>) -> FuseResult<()> {
        writer
            .write_val(&self.open_req)
            .map_err(|_| FuseError::BufferTooSmall)
    }

    fn reply_expectation(&self) -> ReplyExpectation {
        ReplyExpectation::payload(size_of::<OpenReply>())
    }

    fn parse_reply(
        payload_len: usize,
        reader: &mut VmReader<'_, Infallible>,
    ) -> FuseResult<Self::Output> {
        read_payload(payload_len, reader)
    }
}

pub struct OpendirOperation {
    open_req: OpenReq,
}

impl OpendirOperation {
    pub fn new(open_req: OpenReq) -> Self {
        Self { open_req }
    }
}

impl FuseOperation for OpendirOperation {
    type Output = OpenReply;

    fn opcode(&self) -> FuseOpcode {
        FuseOpcode::Opendir
    }

    fn body_len(&self) -> usize {
        size_of::<OpenReq>()
    }

    fn write_body(&mut self, writer: &mut VmWriter<'_, Infallible>) -> FuseResult<()> {
        writer.write_val(&self.open_req).unwrap();

        Ok(())
    }

    fn reply_expectation(&self) -> ReplyExpectation {
        ReplyExpectation::payload(size_of::<OpenReply>())
    }

    fn parse_reply(
        payload_len: usize,
        reader: &mut VmReader<'_, Infallible>,
    ) -> FuseResult<Self::Output> {
        read_payload(payload_len, reader)
    }
}
