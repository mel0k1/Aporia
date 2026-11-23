use crate::{FuseGeneration, FuseNodeId};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct Attr {
    ino: u64,
    size: u64,
    blocks: u64,
    atime: u64,
    mtime: u64,
    ctime: u64,
    atimensec: u32,
    mtimensec: u32,
    ctimensec: u32,
    mode: u32,
    nlink: u32,
    uid: u32,
    gid: u32,
    rdev: u32,
    blksize: u32,
    padding: u32,
}

impl Attr {
    pub fn ino(&self) -> u64 {
        self.ino
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn blocks(&self) -> u64 {
        self.blocks
    }

    pub fn atime(&self) -> u64 {
        self.atime
    }

    pub fn mtime(&self) -> u64 {
        self.mtime
    }

    pub fn ctime(&self) -> u64 {
        self.ctime
    }

    pub fn atimensec(&self) -> u32 {
        self.atimensec
    }

    pub fn mtimensec(&self) -> u32 {
        self.mtimensec
    }

    pub fn ctimensec(&self) -> u32 {
        self.ctimensec
    }

    pub fn mode(&self) -> u32 {
        self.mode
    }

    pub fn nlink(&self) -> u32 {
        self.nlink
    }

    pub fn uid(&self) -> u32 {
        self.uid
    }

    pub fn gid(&self) -> u32 {
        self.gid
    }

    pub fn rdev(&self) -> u32 {
        self.rdev
    }

    pub fn blksize(&self) -> u32 {
        self.blksize
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct EntryReply {
    nodeid: FuseNodeId,
    generation: FuseGeneration,
    entry_valid: u64,
    attr_valid: u64,
    entry_valid_nsec: u32,
    attr_valid_nsec: u32,
    attr: Attr,
}

impl EntryReply {
    pub const fn new(
        nodeid: FuseNodeId,
        generation: FuseGeneration,
        entry_valid: u64,
        attr_valid: u64,
        entry_valid_nsec: u32,
        attr_valid_nsec: u32,
        attr: Attr,
    ) -> Self {
        Self {
            nodeid,
            generation,
            entry_valid,
            attr_valid,
            entry_valid_nsec,
            attr_valid_nsec,
            attr,
        }
    }

    pub fn nodeid(&self) -> FuseNodeId {
        self.nodeid
    }

    pub fn generation(&self) -> FuseGeneration {
        self.generation
    }

    pub fn entry_valid(&self) -> u64 {
        self.entry_valid
    }

    pub fn attr_valid(&self) -> u64 {
        self.attr_valid
    }

    pub fn entry_valid_nsec(&self) -> u32 {
        self.entry_valid_nsec
    }

    pub fn attr_valid_nsec(&self) -> u32 {
        self.attr_valid_nsec
    }

    pub fn attr(&self) -> Attr {
        self.attr
    }
}
