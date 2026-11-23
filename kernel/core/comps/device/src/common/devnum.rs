use core::fmt;

use device_id::DeviceId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DevKind {
    Char,
    Block,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DevNum {
    kind: DevKind,
    id: DeviceId,
}

impl DevNum {
    pub fn char(id: DeviceId) -> Self {
        Self {
            kind: DevKind::Char,
            id,
        }
    }

    pub fn block(id: DeviceId) -> Self {
        Self {
            kind: DevKind::Block,
            id,
        }
    }

    pub fn kind(&self) -> DevKind {
        self.kind
    }

    pub fn id(&self) -> DeviceId {
        self.id
    }
}

pub const DEFAULT_DEVNODE_MODE: u16 = 0o600;

impl fmt::Display for DevNum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.id.major().get(), self.id.minor().get())
    }
}
