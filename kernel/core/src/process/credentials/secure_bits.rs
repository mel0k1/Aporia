use core::sync::atomic::{AtomicU16, Ordering};

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;
use bitflags::bitflags;

use crate::prelude::*;

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub(crate) struct SecureBits: u16 {
        const NOROOT                      = 1 << 0;
        const NOROOT_LOCKED               = 1 << 1;

        const NO_SETUID_FIXUP             = 1 << 2;
        const NO_SETUID_FIXUP_LOCKED      = 1 << 3;

        const KEEP_CAPS                   = 1 << 4;
        const KEEP_CAPS_LOCKED            = 1 << 5;

        const NO_CAP_AMBIENT_RAISE        = 1 << 6;
        const NO_CAP_AMBIENT_RAISE_LOCKED = 1 << 7;
    }
}

impl SecureBits {
    const LOCK_MASK: u16 = 0b10101010;

    pub(super) const fn new_empty() -> Self {
        SecureBits::empty()
    }

    pub(super) const fn locked_bits(&self) -> SecureBits {
        Self::from_bits_truncate((self.bits() & Self::LOCK_MASK) >> 1)
    }

    pub(super) fn no_root(&self) -> bool {
        self.contains(SecureBits::NOROOT)
    }

    pub(super) fn keep_capabilities(&self) -> bool {
        self.contains(SecureBits::KEEP_CAPS)
    }

    pub(super) fn no_setuid_fixup(&self) -> bool {
        self.contains(SecureBits::NO_SETUID_FIXUP)
    }

    pub(super) fn no_cap_ambient_raise(&self) -> bool {
        self.contains(SecureBits::NO_CAP_AMBIENT_RAISE)
    }
}

impl TryFrom<u16> for SecureBits {
    type Error = Error;

    fn try_from(value: u16) -> Result<Self> {
        Self::from_bits(value).ok_or_else(|| errno!(EPERM, "the bits are not valid secure bits"))
    }
}

impl From<SecureBits> for u16 {
    fn from(value: SecureBits) -> Self {
        value.bits()
    }
}

define_atomic_version_of_integer_like_type!(SecureBits, try_from = true, {
    #[derive(Debug)]
    struct AtomicSecureBitsInner(AtomicU16);
});

impl Clone for AtomicSecureBitsInner {
    fn clone(&self) -> Self {
        Self::new(self.load(Ordering::Relaxed))
    }
}

#[derive(Clone, Debug)]
pub(super) struct AtomicSecureBits {
    inner: AtomicSecureBitsInner,
}

impl AtomicSecureBits {
    pub(super) fn new(bits: SecureBits) -> Self {
        Self {
            inner: AtomicSecureBitsInner::new(bits),
        }
    }

    pub(super) fn load(&self, ordering: Ordering) -> SecureBits {
        self.inner.load(ordering)
    }

    pub(super) fn try_store(&self, bits: SecureBits, ordering: Ordering) -> Result<()> {
        let current = self.inner.load(Ordering::Relaxed);
        let locked_bits = current.locked_bits();

        if locked_bits & current != locked_bits & bits {
            return Err(errno!(EPERM, "one or more secure bits are locked"));
        }

        if SecureBits::LOCK_MASK & current.bits() & !bits.bits() != 0 {
            return Err(errno!(EPERM, "locked secure bits cannot be unlocked"));
        }

        self.inner.store(bits, ordering);

        Ok(())
    }
}
