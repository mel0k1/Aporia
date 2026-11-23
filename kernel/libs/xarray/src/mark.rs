use core::sync::atomic::{AtomicU64, Ordering};

use crate::XLockGuard;

#[derive(Debug)]
pub(super) struct Mark {
    inner: AtomicU64,
}

impl Mark {
    pub(super) const fn new(inner: u64) -> Self {
        Self {
            inner: AtomicU64::new(inner),
        }
    }

    pub(super) const fn new_empty() -> Self {
        Self::new(0)
    }

    pub(super) fn update(&self, _guard: XLockGuard, offset: u8, set: bool) -> bool {
        let old_val = self.inner.load(Ordering::Acquire);

        let new_val = if set {
            old_val | (1 << offset as u64)
        } else {
            old_val & !(1 << offset as u64)
        };
        self.inner.store(new_val, Ordering::Release);

        old_val != new_val
    }

    pub(super) fn is_marked(&self, offset: u8) -> bool {
        self.inner.load(Ordering::Acquire) & (1 << offset as u64) != 0
    }

    pub(super) fn is_clear(&self) -> bool {
        self.inner.load(Ordering::Acquire) == 0
    }

    pub(super) fn next_marked(&self, offset: u8) -> Option<u8> {
        let high_marks = self.inner.load(Ordering::Acquire) >> offset;
        if high_marks == 0 {
            None
        } else {
            Some(offset + (high_marks.trailing_zeros() as u8))
        }
    }
}

pub enum XMark {
    Mark0,
    Mark1,
    Mark2,
}

pub(super) const NUM_MARKS: usize = 4;

pub(super) const PRESENT_MARK: usize = 3;

impl XMark {
    pub(super) fn index(&self) -> usize {
        match self {
            XMark::Mark0 => 0,
            XMark::Mark1 => 1,
            XMark::Mark2 => 2,
        }
    }
}

#[derive(Clone, Copy)]
pub enum NoneMark {}
