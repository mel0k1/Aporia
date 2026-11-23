use bitflags::bitflags;
use ostd::mm::{Infallible, VmReader, VmWriter};

use crate::{FuseError, FuseFileHandle, FuseOpcode, FuseOperation, FuseResult, ReplyExpectation};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct FsyncReq {
    fh: FuseFileHandle,
    fsync_flags: FsyncFlags,
    padding: u32,
}

impl FsyncReq {
    pub const fn new(fh: FuseFileHandle, fsync_flags: FsyncFlags) -> Self {
        Self {
            fh,
            fsync_flags,
            padding: 0,
        }
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct FsyncFlags(u32);

bitflags! {
    impl FsyncFlags: u32 {
        const FDATASYNC = 1 << 0;

        const _         = !0;
    }
}

pub struct FsyncOperation {
    fsync_req: FsyncReq,
}

impl FsyncOperation {
    pub const fn new(fh: FuseFileHandle, fsync_flags: FsyncFlags) -> Self {
        Self {
            fsync_req: FsyncReq::new(fh, fsync_flags),
        }
    }
}

impl FuseOperation for FsyncOperation {
    type Output = ();

    fn opcode(&self) -> FuseOpcode {
        FuseOpcode::Fsync
    }

    fn body_len(&self) -> usize {
        size_of::<FsyncReq>()
    }

    fn write_body(&mut self, writer: &mut VmWriter<'_, Infallible>) -> FuseResult<()> {
        writer
            .write_val(&self.fsync_req)
            .map_err(|_| FuseError::BufferTooSmall)
    }

    fn reply_expectation(&self) -> ReplyExpectation {
        ReplyExpectation::HeaderOnly
    }

    fn parse_reply(
        _payload_len: usize,
        _reader: &mut VmReader<'_, Infallible>,
    ) -> FuseResult<Self::Output> {
        Ok(())
    }
}

pub struct FsyncdirOperation {
    fsync_req: FsyncReq,
}

impl FsyncdirOperation {
    pub const fn new(fh: FuseFileHandle, fsync_flags: FsyncFlags) -> Self {
        Self {
            fsync_req: FsyncReq::new(fh, fsync_flags),
        }
    }
}

impl FuseOperation for FsyncdirOperation {
    type Output = ();

    fn opcode(&self) -> FuseOpcode {
        FuseOpcode::Fsyncdir
    }

    fn body_len(&self) -> usize {
        size_of::<FsyncReq>()
    }

    fn write_body(&mut self, writer: &mut VmWriter<'_, Infallible>) -> FuseResult<()> {
        writer
            .write_val(&self.fsync_req)
            .map_err(|_| FuseError::BufferTooSmall)
    }

    fn reply_expectation(&self) -> ReplyExpectation {
        ReplyExpectation::HeaderOnly
    }

    fn parse_reply(
        _payload_len: usize,
        _reader: &mut VmReader<'_, Infallible>,
    ) -> FuseResult<Self::Output> {
        Ok(())
    }
}
