use core::{
    iter::Step,
    ops::{Add, Sub},
};

use ostd::const_assert;

pub type Bid = BlockId<BLOCK_SIZE>;
pub type Sid = BlockId<SECTOR_SIZE>;

impl From<Bid> for Sid {
    fn from(bid: Bid) -> Self {
        Self::new(bid.to_raw() * (BLOCK_SIZE / SECTOR_SIZE) as u64)
    }
}

const BLOCK_SIZE: u16 = super::BLOCK_SIZE as u16;
const SECTOR_SIZE: u16 = super::SECTOR_SIZE as u16;
const_assert!(BLOCK_SIZE / SECTOR_SIZE >= 1);

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Pod)]
pub struct BlockId<const N: u16>(u64);

impl<const N: u16> BlockId<N> {
    pub const fn new(raw_id: u64) -> Self {
        Self(raw_id)
    }

    pub const fn from_offset(offset: usize) -> Self {
        Self((offset / (N as usize)) as _)
    }

    pub fn to_offset(self) -> usize {
        (self.0 as usize) * (N as usize)
    }

    pub fn to_raw(self) -> u64 {
        self.0
    }
}

impl<const N: u16> Add<u64> for BlockId<N> {
    type Output = Self;

    fn add(self, other: u64) -> Self::Output {
        Self(self.0 + other)
    }
}

impl<const N: u16> Sub<u64> for BlockId<N> {
    type Output = Self;

    fn sub(self, other: u64) -> Self::Output {
        Self(self.0 - other)
    }
}

impl<const N: u16> Step for BlockId<N> {
    fn steps_between(start: &Self, end: &Self) -> (usize, Option<usize>) {
        u64::steps_between(&start.0, &end.0)
    }

    fn forward_checked(start: Self, count: usize) -> Option<Self> {
        u64::forward_checked(start.0, count).map(Self::new)
    }

    fn forward_overflowing(start: Self, count: usize) -> (Self, bool) {
        let (next, overflow) = u64::forward_overflowing(start.0, count);
        (Self::new(next), overflow)
    }

    fn backward_checked(start: Self, count: usize) -> Option<Self> {
        u64::backward_checked(start.0, count).map(Self::new)
    }

    fn backward_overflowing(start: Self, count: usize) -> (Self, bool) {
        let (prev, overflow) = u64::backward_overflowing(start.0, count);
        (Self::new(prev), overflow)
    }
}
