#![no_std]

use core::ops::{Deref, DerefMut};

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub struct Rights: u32 {
        const DUP    = 1 << 0;
        const READ   = 1 << 1;
        const WRITE  = 1 << 2;
        const EXEC   = 1 << 3;
        const SIGNAL = 1 << 7;
    }
}

typeflags::typeflags! {
    pub trait TRights: u32 {
        pub struct Dup     = Rights::DUP.bits();
        pub struct Read     = Rights::READ.bits();
        pub struct Write    = Rights::WRITE.bits();
        pub struct Exec     = Rights::EXEC.bits();
        pub struct Signal   = Rights::SIGNAL.bits();
    }
}

pub type Full = TRightSet<TRights![Dup, Read, Write, Exec, Signal]>;
pub type ReadOp = TRights![Read];
pub type WriteOp = TRights![Write];
pub type ReadDupOp = TRights![Read, Dup];
pub type ReadWriteOp = TRights![Read, Write];
pub type FullOp = TRights![Read, Write, Dup];

#[derive(Clone, Copy)]
pub struct TRightSet<T>(pub T);

impl<T> Deref for TRightSet<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for TRightSet<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
