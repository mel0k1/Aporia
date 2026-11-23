use core::sync::atomic::{AtomicU64, Ordering};

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod)]
pub struct FuseUnique(u64);

impl FuseUnique {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Pod)]
pub struct FuseFileHandle(u64);

impl FuseFileHandle {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Pod)]
pub struct FuseNodeId(u64);

impl FuseNodeId {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Pod)]
pub struct FuseGeneration(u64);

impl FuseGeneration {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

#[repr(transparent)]
pub struct LookupCount(AtomicU64);

impl LookupCount {
    pub const fn initial() -> Self {
        Self(AtomicU64::new(1))
    }

    pub fn acquire(&self) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }

    pub fn drain(&self) -> u64 {
        self.0.swap(0, Ordering::AcqRel)
    }
}
