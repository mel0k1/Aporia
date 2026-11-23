use core::num::NonZeroUsize;

use int_to_c_enum::TryFromInt;
use ostd::mm::{Infallible, VmReader, VmWriter};

use crate::FuseResult;

pub trait FuseOperation {
    type Output;

    fn opcode(&self) -> FuseOpcode;

    fn body_len(&self) -> usize {
        0
    }

    fn write_body(&mut self, _writer: &mut VmWriter<'_, Infallible>) -> FuseResult<()> {
        Ok(())
    }

    fn reply_expectation(&self) -> ReplyExpectation;

    fn parse_reply(
        payload_len: usize,
        reader: &mut VmReader<'_, Infallible>,
    ) -> FuseResult<Self::Output>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplyExpectation {
    None,
    HeaderOnly,
    FixedPayload(NonZeroUsize),
    VariablePayload(NonZeroUsize),
    WritePayload { requested_size: usize },
}

impl ReplyExpectation {
    pub fn payload(payload_size: usize) -> Self {
        let Some(payload_size) = NonZeroUsize::new(payload_size) else {
            return Self::HeaderOnly;
        };
        Self::FixedPayload(payload_size)
    }

    pub fn variable_payload(max_payload_size: usize) -> Self {
        let Some(max_payload_size) = NonZeroUsize::new(max_payload_size) else {
            return Self::HeaderOnly;
        };
        Self::VariablePayload(max_payload_size)
    }
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub enum FuseOpcode {
    Lookup = 1,
    Forget = 2,
    Getattr = 3,
    Setattr = 4,
    Readlink = 5,
    Symlink = 6,
    Mknod = 8,
    Mkdir = 9,
    Unlink = 10,
    Rmdir = 11,
    Rename = 12,
    Link = 13,
    Open = 14,
    Read = 15,
    Write = 16,
    Statfs = 17,
    Release = 18,
    Fsync = 20,
    Setxattr = 21,
    Getxattr = 22,
    Listxattr = 23,
    Removexattr = 24,
    Flush = 25,
    Init = 26,
    Opendir = 27,
    Readdir = 28,
    Releasedir = 29,
    Fsyncdir = 30,
    Getlk = 31,
    Setlk = 32,
    Setlkw = 33,
    Access = 34,
    Create = 35,
    Interrupt = 36,
    Bmap = 37,
    Destroy = 38,
    Ioctl = 39,
    Poll = 40,
    NotifyReply = 41,
    BatchForget = 42,
    Fallocate = 43,
    Readdirplus = 44,
    Rename2 = 45,
    Lseek = 46,
    CopyFileRange = 47,
    SetupMapping = 48,
    RemoveMapping = 49,
    SyncFs = 50,
    Tmpfile = 51,
}
