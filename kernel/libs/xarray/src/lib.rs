#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use core::marker::PhantomData;

pub use cursor::{Cursor, CursorMut, SetMarkError};
use entry::NodeEntry;
use mark::NoneMark;
pub use mark::XMark;
use ostd::{
    sync::{
        LocalIrqDisabled, PreemptDisabled, RcuOption, SpinGuardian, SpinLock, SpinLockGuard,
        non_null::NonNullPtr,
    },
    task::atomic_mode::{AsAtomicModeGuard, InAtomicMode},
};
pub use range::Range;

mod cursor;
mod entry;
mod mark;
mod node;
mod range;

#[cfg(ktest)]
mod test;

const BITS_PER_LAYER: usize = 6;
const SLOT_SIZE: usize = 1 << BITS_PER_LAYER;
const SLOT_MASK: usize = SLOT_SIZE - 1;

pub struct XArray<P, M = NoneMark>
where
    P: NonNullPtr + Send + Sync,
{
    head: RcuOption<NodeEntry<P>>,
    xlock: SpinLock<()>,
    _marker: PhantomData<M>,
}

#[derive(Clone, Copy)]
struct XLockGuard<'a>(&'a dyn InAtomicMode);

impl<P: NonNullPtr + Send + Sync, M> Default for XArray<P, M> {
    fn default() -> Self {
        Self::new()
    }
}

impl<P: NonNullPtr + Send + Sync, M> XArray<P, M> {
    pub const fn new() -> Self {
        Self {
            head: RcuOption::new_none(),
            xlock: SpinLock::new(()),
            _marker: PhantomData,
        }
    }

    pub fn lock(&self) -> LockedXArray<'_, P, M> {
        LockedXArray {
            xa: self,
            guard: self.xlock.lock(),
            _marker: PhantomData,
        }
    }

    pub fn lock_irq_disabled(&self) -> LockedXArray<'_, P, M, LocalIrqDisabled> {
        LockedXArray {
            xa: self,
            guard: self.xlock.disable_irq().lock(),
            _marker: PhantomData,
        }
    }

    pub fn cursor<'a, G: AsAtomicModeGuard>(
        &'a self,
        guard: &'a G,
        index: u64,
    ) -> Cursor<'a, P, M> {
        Cursor::new(self, guard, index)
    }

    pub fn range<'a, G: AsAtomicModeGuard>(
        &'a self,
        guard: &'a G,
        range: core::ops::Range<u64>,
    ) -> Range<'a, P, M> {
        let cursor = self.cursor(guard, range.start);
        Range::new(cursor, range.end)
    }

    pub fn load<'a, G: AsAtomicModeGuard>(
        &'a self,
        guard: &'a G,
        index: u64,
    ) -> Option<P::Ref<'a>> {
        let mut cursor = self.cursor(guard, index);
        cursor.load()
    }
}

impl<P: NonNullPtr + Sync + Send, M> Drop for XArray<P, M> {
    fn drop(&mut self) {
        self.lock().clear();
    }
}

pub struct LockedXArray<'a, P, M = NoneMark, G = PreemptDisabled>
where
    P: NonNullPtr + Send + Sync,
    G: SpinGuardian,
{
    xa: &'a XArray<P, M>,
    guard: SpinLockGuard<'a, (), G>,
    _marker: PhantomData<(P, M)>,
}

impl<P: NonNullPtr + Send + Sync, M, G: SpinGuardian> LockedXArray<'_, P, M, G> {
    pub fn clear(&mut self) {
        if let Some(head) = self.xa.head.read_with(&self.guard) {
            head.clear_parent(XLockGuard(self.guard.as_atomic_mode_guard()));
        }

        self.xa.head.update(None);
    }

    pub fn cursor_mut(&mut self, index: u64) -> CursorMut<'_, P, M> {
        CursorMut::new(self.xa, &self.guard, index)
    }

    pub fn store(&mut self, index: u64, item: P) {
        let mut cursor = self.cursor_mut(index);
        cursor.store(item)
    }

    pub fn remove(&mut self, index: u64) -> Option<P::Ref<'_>> {
        let mut cursor = self.cursor_mut(index);
        cursor.remove()
    }

    pub fn cursor(&self, index: u64) -> Cursor<'_, P, M> {
        Cursor::new(self.xa, &self.guard, index)
    }

    pub fn range(&self, range: core::ops::Range<u64>) -> Range<'_, P, M> {
        let cursor = self.cursor(range.start);
        Range::new(cursor, range.end)
    }

    pub fn load(&self, index: u64) -> Option<P::Ref<'_>> {
        let mut cursor = self.cursor(index);
        cursor.load()
    }
}
