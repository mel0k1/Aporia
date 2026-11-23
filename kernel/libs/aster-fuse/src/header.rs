use crate::{FuseNodeId, FuseUnique};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct ReqHeader {
    len: u32,
    opcode: u32,
    unique: FuseUnique,
    nodeid: FuseNodeId,
    uid: u32,
    gid: u32,
    pid: u32,
    total_extlen: u16,
    padding: u16,
}

impl ReqHeader {
    pub const fn new(len: u32, opcode: u32, unique: FuseUnique, nodeid: FuseNodeId) -> Self {
        Self {
            len,
            opcode,
            unique,
            nodeid,
            uid: 0,
            gid: 0,
            pid: 0,
            total_extlen: 0,
            padding: 0,
        }
    }

    pub fn len(&self) -> u32 {
        self.len
    }

    pub fn opcode(&self) -> u32 {
        self.opcode
    }

    pub fn unique(&self) -> FuseUnique {
        self.unique
    }

    pub fn nodeid(&self) -> FuseNodeId {
        self.nodeid
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct ReplyHeader {
    len: u32,
    error: i32,
    unique: FuseUnique,
}

impl ReplyHeader {
    pub const fn new(len: u32, error: i32, unique: FuseUnique) -> Self {
        Self { len, error, unique }
    }

    pub const fn empty() -> Self {
        Self::new(0, 0, FuseUnique::new(0))
    }

    pub fn len(&self) -> u32 {
        self.len
    }

    pub fn error(&self) -> i32 {
        self.error
    }

    pub fn unique(&self) -> FuseUnique {
        self.unique
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}
