use core::fmt::Debug;

use bitflags::bitflags;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageProperty {
    pub flags: PageFlags,
    pub cache: CachePolicy,
    pub(crate) priv_flags: PrivilegedPageFlags,
}

impl PageProperty {
    pub fn new_user(flags: PageFlags, cache: CachePolicy) -> Self {
        Self {
            flags,
            cache,
            priv_flags: PrivilegedPageFlags::USER,
        }
    }

    pub fn new_guest(flags: PageFlags, cache: CachePolicy) -> Self {
        Self::new_user(flags, cache)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CachePolicy {
    Uncacheable,
    WriteCombining,
    WriteProtected,
    Writethrough,
    Writeback,
}

bitflags! {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct PageFlags: u8 {
        const R        = 0b00000001;
        const W        = 0b00000010;
        const X        = 0b00000100;
        const RW       = Self::R.bits() | Self::W.bits();
        const RX       = Self::R.bits() | Self::X.bits();
        const RWX      = Self::R.bits() | Self::W.bits() | Self::X.bits();

        const ACCESSED = 0b00001000;
        const DIRTY    = 0b00010000;

        const AVAIL2   = 0b10000000;
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(crate) struct PrivilegedPageFlags: u8 {
        const USER   = 0b00000001;
        const GLOBAL = 0b00000010;

        const AVAIL1 = 0b01000000;

        #[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
        const SHARED = 0b10000000;
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(crate) struct PageTableFlags: u8 {
        const AVAIL1 = 0b01000000;
        const AVAIL2 = 0b10000000;
    }
}
