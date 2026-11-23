use bitflags::bitflags;
use ostd::mm::{Infallible, VmReader, VmWriter};

use crate::{FuseError, FuseFileHandle, FuseOpcode, FuseOperation, FuseResult, ReplyExpectation};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct ReleaseReq {
    fh: FuseFileHandle,
    flags: u32,
    release_flags: ReleaseFlags,
    lock_owner: u64,
}

impl ReleaseReq {
    pub const fn new(fh: FuseFileHandle, flags: u32, release_flags: ReleaseFlags) -> Self {
        Self {
            fh,
            flags,
            release_flags,
            lock_owner: 0,
        }
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct ReleaseFlags(u32);

bitflags! {
    impl ReleaseFlags: u32 {
        const RELEASE_FLUSH        = 1 << 0;
        const RELEASE_FLOCK_UNLOCK = 1 << 1;

        const _                    = !0;
    }
}

pub struct ReleaseOperation {
    release_req: ReleaseReq,
    release_options: ReleaseOptions,
}

impl ReleaseOperation {
    pub fn new(fh: FuseFileHandle, flags: u32, release_options: ReleaseOptions) -> Self {
        Self {
            release_req: ReleaseReq::new(fh, flags, release_options.flags),
            release_options,
        }
    }
}

impl FuseOperation for ReleaseOperation {
    type Output = ();

    fn opcode(&self) -> FuseOpcode {
        self.release_options.kind.opcode()
    }

    fn body_len(&self) -> usize {
        size_of::<ReleaseReq>()
    }

    fn write_body(&mut self, writer: &mut VmWriter<'_, Infallible>) -> FuseResult<()> {
        writer
            .write_val(&self.release_req)
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

#[derive(Clone, Copy, Debug)]
pub struct ReleaseOptions {
    kind: ReleaseKind,
    flags: ReleaseFlags,
}

impl ReleaseOptions {
    pub const fn new(kind: ReleaseKind, flags: ReleaseFlags) -> Self {
        Self { kind, flags }
    }

    pub const fn kind(self) -> ReleaseKind {
        self.kind
    }

    pub const fn flags(self) -> ReleaseFlags {
        self.flags
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReleaseKind {
    File,
    Directory,
}

impl ReleaseKind {
    fn opcode(self) -> FuseOpcode {
        match self {
            Self::File => FuseOpcode::Release,
            Self::Directory => FuseOpcode::Releasedir,
        }
    }
}
