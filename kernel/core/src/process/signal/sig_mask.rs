use core::{
    fmt::LowerHex,
    ops,
    sync::atomic::{AtomicU64, Ordering},
};

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;

use super::{constants::MIN_STD_SIG_NUM, sig_num::SigNum};
use crate::prelude::*;

pub(crate) type SigMask = SigSet;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SigSet {
    bits: u64,
}

impl From<SigNum> for SigSet {
    fn from(signum: SigNum) -> Self {
        let idx = signum.as_u8() - MIN_STD_SIG_NUM;
        Self { bits: 1_u64 << idx }
    }
}

impl From<u64> for SigSet {
    fn from(bits: u64) -> Self {
        SigSet { bits }
    }
}

impl From<SigSet> for u64 {
    fn from(set: SigSet) -> u64 {
        set.bits
    }
}

impl<T: Into<SigSet>> ops::BitAnd<T> for SigSet {
    type Output = Self;

    fn bitand(self, rhs: T) -> Self {
        SigSet {
            bits: self.bits & rhs.into().bits,
        }
    }
}

impl<T: Into<SigSet>> ops::BitAndAssign<T> for SigSet {
    fn bitand_assign(&mut self, rhs: T) {
        self.bits &= rhs.into().bits;
    }
}

impl<T: Into<SigSet>> ops::BitOr<T> for SigSet {
    type Output = Self;

    fn bitor(self, rhs: T) -> Self {
        SigSet {
            bits: self.bits | rhs.into().bits,
        }
    }
}

impl<T: Into<SigSet>> ops::BitOrAssign<T> for SigSet {
    fn bitor_assign(&mut self, rhs: T) {
        self.bits |= rhs.into().bits;
    }
}

#[expect(clippy::suspicious_arithmetic_impl)]
impl<T: Into<SigSet>> ops::Add<T> for SigSet {
    type Output = Self;

    fn add(self, rhs: T) -> Self {
        SigSet {
            bits: self.bits | rhs.into().bits,
        }
    }
}

#[expect(clippy::suspicious_op_assign_impl)]
impl<T: Into<SigSet>> ops::AddAssign<T> for SigSet {
    fn add_assign(&mut self, rhs: T) {
        self.bits |= rhs.into().bits;
    }
}

impl<T: Into<SigSet>> ops::Sub<T> for SigSet {
    type Output = Self;

    fn sub(self, rhs: T) -> Self {
        SigSet {
            bits: self.bits & !rhs.into().bits,
        }
    }
}

impl<T: Into<SigSet>> ops::SubAssign<T> for SigSet {
    fn sub_assign(&mut self, rhs: T) {
        self.bits &= !rhs.into().bits;
    }
}

impl ops::Not for SigSet {
    type Output = Self;

    fn not(self) -> Self {
        SigSet { bits: !self.bits }
    }
}

impl LowerHex for SigSet {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        LowerHex::fmt(&self.bits, f)
    }
}

impl SigSet {
    pub(crate) fn new_empty() -> Self {
        SigSet { bits: 0 }
    }

    pub(crate) fn new_full() -> Self {
        SigSet { bits: !0 }
    }

    #[expect(dead_code)]
    pub(crate) const fn is_empty(&self) -> bool {
        self.bits == 0
    }

    #[expect(dead_code)]
    pub(crate) const fn is_full(&self) -> bool {
        self.bits == !0
    }

    #[expect(dead_code)]
    pub(crate) fn count(&self) -> usize {
        self.bits.count_ones() as usize
    }

    pub(crate) fn contains(&self, other: impl Into<Self>) -> bool {
        let other = other.into();
        self.bits & other.bits == other.bits
    }

    pub(crate) fn intersects(&self, other: impl Into<Self>) -> bool {
        let other = other.into();
        self.bits & other.bits != 0
    }
}

pub(crate) type AtomicSigMask = AtomicSigSet;

define_atomic_version_of_integer_like_type!(SigSet, {
    pub(crate) struct AtomicSigSet(AtomicU64);
});

impl From<SigSet> for AtomicSigSet {
    fn from(set: SigSet) -> Self {
        Self::new(set)
    }
}

impl AtomicSigSet {
    pub(crate) fn new_empty() -> Self {
        AtomicSigSet::new(0)
    }

    pub(crate) fn contains(&self, signals: impl Into<SigSet>, ordering: Ordering) -> bool {
        self.load(ordering).contains(signals.into())
    }
}
