use bitflags::bitflags;
use ostd::mm::{Infallible, VmReader, VmWriter};
use ostd_pod::{FromZeros, IntoBytes};

use super::util::read_bytes;
use crate::{FuseError, FuseOpcode, FuseOperation, FuseResult, ReplyExpectation};

const FUSE_COMPAT_INIT_OUT_SIZE: usize = 8;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct InitReq {
    major: u32,
    minor: u32,
    max_readahead: u32,
    flags: FuseInitFlags,
    flags2: FuseInitFlags2,
    unused: [u32; 11],
}

impl InitReq {
    pub const fn new(
        major: u32,
        minor: u32,
        max_readahead: u32,
        flags: FuseInitFlags,
        flags2: FuseInitFlags2,
    ) -> Self {
        Self {
            major,
            minor,
            max_readahead,
            flags,
            flags2,
            unused: [0; 11],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct InitReply {
    major: u32,
    minor: u32,
    max_readahead: u32,
    flags: FuseInitFlags,
    max_background: u16,
    congestion_threshold: u16,
    max_write: u32,
    time_gran: u32,
    max_pages: u16,
    map_alignment: u16,
    flags2: FuseInitFlags2,
    max_stack_depth: u32,
    request_timeout: u16,
    unused: [u16; 11],
}

impl InitReply {
    pub fn major(&self) -> u32 {
        self.major
    }

    pub fn minor(&self) -> u32 {
        self.minor
    }

    pub fn max_readahead(&self) -> u32 {
        self.max_readahead
    }

    pub fn flags(&self) -> FuseInitFlags {
        self.flags
    }

    pub fn max_write(&self) -> u32 {
        self.max_write
    }

    pub fn time_gran(&self) -> u32 {
        self.time_gran
    }

    pub fn max_pages(&self) -> u16 {
        self.max_pages
    }

    pub fn map_alignment(&self) -> u16 {
        self.map_alignment
    }

    pub fn flags2(&self) -> FuseInitFlags2 {
        self.flags2
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct FuseInitFlags(u32);

bitflags! {
    impl FuseInitFlags: u32 {
        const ASYNC_READ          = 1 << 0;
        const POSIX_LOCKS         = 1 << 1;
        const FILE_OPS            = 1 << 2;
        const ATOMIC_O_TRUNC      = 1 << 3;
        const EXPORT_SUPPORT      = 1 << 4;
        const BIG_WRITES          = 1 << 5;
        const DONT_MASK           = 1 << 6;
        const SPLICE_WRITE        = 1 << 7;
        const SPLICE_MOVE         = 1 << 8;
        const SPLICE_READ         = 1 << 9;
        const FLOCK_LOCKS         = 1 << 10;
        const HAS_IOCTL_DIR       = 1 << 11;
        const AUTO_INVAL_DATA     = 1 << 12;
        const DO_READDIRPLUS      = 1 << 13;
        const READDIRPLUS_AUTO    = 1 << 14;
        const ASYNC_DIO           = 1 << 15;
        const WRITEBACK_CACHE     = 1 << 16;
        const NO_OPEN_SUPPORT     = 1 << 17;
        const PARALLEL_DIROPS     = 1 << 18;
        const HANDLE_KILLPRIV     = 1 << 19;
        const POSIX_ACL           = 1 << 20;
        const ABORT_ERROR         = 1 << 21;
        const MAX_PAGES           = 1 << 22;
        const CACHE_SYMLINKS      = 1 << 23;
        const NO_OPENDIR_SUPPORT  = 1 << 24;
        const EXPLICIT_INVAL_DATA = 1 << 25;
        const MAP_ALIGNMENT       = 1 << 26;
        const SUBMOUNTS           = 1 << 27;
        const HANDLE_KILLPRIV_V2  = 1 << 28;
        const SETXATTR_EXT        = 1 << 29;
        const INIT_EXT            = 1 << 30;
        const INIT_RESERVED       = 1 << 31;

        const _                   = !0;
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct FuseInitFlags2(u32);

bitflags! {
    impl FuseInitFlags2: u32 {
        const SECURITY_CTX         = 1 << 0;
        const HAS_INODE_DAX        = 1 << 1;
        const CREATE_SUPP_GROUP    = 1 << 2;
        const HAS_EXPIRE_ONLY      = 1 << 3;
        const DIRECT_IO_ALLOW_MMAP = 1 << 4;
        const PASSTHROUGH          = 1 << 5;
        const NO_EXPORT_SUPPORT    = 1 << 6;
        const HAS_RESEND           = 1 << 7;
        const ALLOW_IDMAP          = 1 << 8;
        const OVER_IO_URING        = 1 << 9;
        const REQUEST_TIMEOUT      = 1 << 10;

        const _                    = !0;
    }
}

pub struct InitOperation {
    init_req: InitReq,
}

impl InitOperation {
    pub fn new(init_req: InitReq) -> Self {
        Self { init_req }
    }
}

impl FuseOperation for InitOperation {
    type Output = InitReply;

    fn opcode(&self) -> FuseOpcode {
        FuseOpcode::Init
    }

    fn body_len(&self) -> usize {
        size_of::<InitReq>()
    }

    fn write_body(&mut self, writer: &mut VmWriter<'_, Infallible>) -> FuseResult<()> {
        writer
            .write_val(&self.init_req)
            .map_err(|_| FuseError::BufferTooSmall)
    }

    fn reply_expectation(&self) -> ReplyExpectation {
        ReplyExpectation::variable_payload(size_of::<InitReply>())
    }

    fn parse_reply(
        payload_len: usize,
        reader: &mut VmReader<'_, Infallible>,
    ) -> FuseResult<Self::Output> {
        if payload_len < FUSE_COMPAT_INIT_OUT_SIZE {
            return Err(FuseError::MalformedResponse);
        }

        let mut init_reply = InitReply::new_zeroed();
        let read_len = core::cmp::min(payload_len, size_of::<InitReply>());
        read_bytes(reader, &mut init_reply.as_mut_bytes()[..read_len])?;

        Ok(init_reply)
    }
}
