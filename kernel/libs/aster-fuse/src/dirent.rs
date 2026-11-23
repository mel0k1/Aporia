use alloc::string::String;

use int_to_c_enum::TryFromInt;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct Dirent {
    ino: u64,
    off: u64,
    namelen: u32,
    typ: u32,
}

impl Dirent {
    pub fn ino(&self) -> u64 {
        self.ino
    }

    pub fn off(&self) -> u64 {
        self.off
    }

    pub fn namelen(&self) -> u32 {
        self.namelen
    }

    pub fn typ(&self) -> u32 {
        self.typ
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirOffset(u64);

impl DirOffset {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone)]
pub struct FuseDirEntry {
    ino: u64,
    offset: DirOffset,
    type_: DirentType,
    name: String,
}

impl FuseDirEntry {
    pub fn new(ino: u64, offset: DirOffset, type_: DirentType, name: String) -> Self {
        Self {
            ino,
            offset,
            type_,
            name,
        }
    }

    pub fn ino(&self) -> u64 {
        self.ino
    }

    pub fn offset(&self) -> DirOffset {
        self.offset
    }

    pub fn type_(&self) -> DirentType {
        self.type_
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub enum DirentType {
    Unknown = 0,
    Fifo = 1,
    Char = 2,
    Dir = 4,
    Block = 6,
    Regular = 8,
    Link = 10,
    Sock = 12,
}
