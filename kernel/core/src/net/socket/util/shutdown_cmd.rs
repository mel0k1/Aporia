use crate::prelude::*;

#[expect(non_camel_case_types)]
#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub(crate) enum SockShutdownCmd {
    SHUT_RD = 0,
    SHUT_WR = 1,
    SHUT_RDWR = 2,
}

impl SockShutdownCmd {
    pub(crate) fn shut_read(&self) -> bool {
        *self == Self::SHUT_RD || *self == Self::SHUT_RDWR
    }

    pub(crate) fn shut_write(&self) -> bool {
        *self == Self::SHUT_WR || *self == Self::SHUT_RDWR
    }
}
