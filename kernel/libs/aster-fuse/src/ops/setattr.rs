use bitflags::bitflags;
use ostd::mm::{Infallible, VmReader, VmWriter};

use super::util::read_payload;
use crate::{
    FuseAttrReply, FuseError, FuseFileHandle, FuseOpcode, FuseOperation, FuseResult,
    ReplyExpectation,
};

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct SetattrValid(u32);

bitflags! {
    impl SetattrValid: u32 {
        const FATTR_MODE      = 1 << 0;
        const FATTR_UID       = 1 << 1;
        const FATTR_GID       = 1 << 2;
        const FATTR_SIZE      = 1 << 3;
        const FATTR_ATIME     = 1 << 4;
        const FATTR_MTIME     = 1 << 5;
        const FATTR_FH        = 1 << 6;
        const FATTR_ATIME_NOW = 1 << 7;
        const FATTR_MTIME_NOW = 1 << 8;
        const FATTR_LOCKOWNER = 1 << 9;
        const FATTR_CTIME     = 1 << 10;

        const _               = !0;
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct SetattrReq {
    valid: SetattrValid,
    padding: u32,
    fh: FuseFileHandle,
    size: u64,
    lock_owner: u64,
    atime: u64,
    mtime: u64,
    ctime: u64,
    atimensec: u32,
    mtimensec: u32,
    ctimensec: u32,
    mode: u32,
    unused4: u32,
    uid: u32,
    gid: u32,
    unused5: u32,
}

impl SetattrReq {
    pub const fn new(valid: SetattrValid) -> Self {
        Self {
            valid,
            padding: 0,
            fh: FuseFileHandle::new(0),
            size: 0,
            lock_owner: 0,
            atime: 0,
            mtime: 0,
            ctime: 0,
            atimensec: 0,
            mtimensec: 0,
            ctimensec: 0,
            mode: 0,
            unused4: 0,
            uid: 0,
            gid: 0,
            unused5: 0,
        }
    }

    const fn set_field_valid(mut self, valid: SetattrValid) -> Self {
        self.valid = self.valid.union(valid);
        self
    }

    pub fn valid(&self) -> SetattrValid {
        self.valid
    }

    pub const fn set_fh(mut self, fh: FuseFileHandle) -> Self {
        self.fh = fh;
        self.set_field_valid(SetattrValid::FATTR_FH)
    }

    pub const fn set_size(mut self, size: u64) -> Self {
        self.size = size;
        self.set_field_valid(SetattrValid::FATTR_SIZE)
    }

    pub const fn set_mode(mut self, mode: u32) -> Self {
        self.mode = mode;
        self.set_field_valid(SetattrValid::FATTR_MODE)
    }

    pub const fn set_uid(mut self, uid: u32) -> Self {
        self.uid = uid;
        self.set_field_valid(SetattrValid::FATTR_UID)
    }

    pub const fn set_gid(mut self, gid: u32) -> Self {
        self.gid = gid;
        self.set_field_valid(SetattrValid::FATTR_GID)
    }

    pub const fn set_atime(mut self, atime: u64, atimensec: u32) -> Self {
        self.atime = atime;
        self.atimensec = atimensec;
        self.set_field_valid(SetattrValid::FATTR_ATIME)
    }

    pub const fn set_mtime(mut self, mtime: u64, mtimensec: u32) -> Self {
        self.mtime = mtime;
        self.mtimensec = mtimensec;
        self.set_field_valid(SetattrValid::FATTR_MTIME)
    }

    pub const fn set_ctime(mut self, ctime: u64, ctimensec: u32) -> Self {
        self.ctime = ctime;
        self.ctimensec = ctimensec;
        self.set_field_valid(SetattrValid::FATTR_CTIME)
    }
}

pub struct SetattrOperation {
    setattr_req: SetattrReq,
}

impl SetattrOperation {
    pub fn new(setattr_req: SetattrReq) -> Self {
        Self { setattr_req }
    }
}

impl FuseOperation for SetattrOperation {
    type Output = FuseAttrReply;

    fn opcode(&self) -> FuseOpcode {
        FuseOpcode::Setattr
    }

    fn body_len(&self) -> usize {
        size_of::<SetattrReq>()
    }

    fn write_body(&mut self, writer: &mut VmWriter<'_, Infallible>) -> FuseResult<()> {
        writer
            .write_val(&self.setattr_req)
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
