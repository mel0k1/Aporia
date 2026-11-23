use crate::{
    ioc,
    util::ioctl::{InData, OutData, PassByVal},
};

pub(super) type SetPtyLock   = ioc!(TIOCSPTLCK,  b'T', 0x31, InData<i32>);
pub(super) type GetPtyLock   = ioc!(TIOCGPTLCK,  b'T', 0x39, OutData<i32>);

pub(super) type OpenPtySlave = ioc!(TIOCGPTPEER, 0x5441,     InData<u32, PassByVal>);

pub(super) type SetPktMode   = ioc!(TIOCPKT,     0x5420,     InData<i32>);
pub(super) type GetPktMode   = ioc!(TIOCGPKT,    b'T', 0x38, OutData<i32>);

pub(super) type GetPtyNumber = ioc!(TIOCGPTN,    b'T', 0x30, OutData<u32>);
