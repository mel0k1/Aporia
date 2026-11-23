#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct CVtState {
    pub active: u16,
    pub signal: u16,
    pub state: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct CVtMode {
    pub mode: u8,
    pub waitv: u8,
    pub relsig: u16,
    pub acqsig: u16,
    pub frsig: u16,
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReleaseDisplayType {
    DenyRelease = 0,
    AllowRelease = 1,
    AckAcquire = 2,
}

impl From<i32> for ReleaseDisplayType {
    fn from(value: i32) -> Self {
        match value {
            0 => Self::DenyRelease,
            2 => Self::AckAcquire,
            _ => Self::AllowRelease,
        }
    }
}
