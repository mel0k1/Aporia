use crate::{
    device::tty::{
        CFontOp,
        vt::c_types::{CVtMode, CVtState},
    },
    ioc,
    util::ioctl::{InData, OutData, PassByVal},
};

pub(super) type GetKeyboardType = ioc!(KDGKBTYPE,       0x4B33,     OutData<u8>);

pub(super) type SetGraphicsMode = ioc!(KDSETMODE,       0x4B3A,     InData<i32, PassByVal>);
pub(super) type GetGraphicsMode = ioc!(KDGETMODE,       0x4B3B,     OutData<i32>);

pub(super) type GetKeyboardMode = ioc!(KDGKBMODE,       0x4B44,     OutData<i32>);
pub(super) type SetKeyboardMode = ioc!(KDSKBMODE,       0x4B45,     InData<i32, PassByVal>);

pub(super) type SetOrGetFont    = ioc!(KDFONTOP,        0x4B72,     InData<CFontOp>);

pub(super) type GetAvailableVt  = ioc!(VT_OPENQRY,      0x5600,     OutData<i32>);

pub(super) type GetVtMode       = ioc!(VT_GETMODE,      0x5601,     OutData<CVtMode>);
pub(super) type SetVtMode       = ioc!(VT_SETMODE,      0x5602,     InData<CVtMode>);

pub(super) type GetVtState      = ioc!(VT_GETSTATE,     0x5603,     OutData<CVtState>);

pub(super) type ReleaseDisplay  = ioc!(VT_RELDISP,      0x5605,     InData<i32, PassByVal>);

pub(super) type ActivateVt      = ioc!(VT_ACTIVATE,     0x5606,     InData<i32, PassByVal>);
pub(super) type WaitForVtActive = ioc!(VT_WAITACTIVE,   0x5607,     InData<i32, PassByVal>);

pub(super) type DisallocateVt   = ioc!(VT_DISALLOCATE,  0x5608,     InData<i32, PassByVal>);
