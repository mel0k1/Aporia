mod locking;

use core::{fmt::Debug, marker::PhantomData, mem::ManuallyDrop, ops::Range};

use align_ext::AlignExt;

use super::{
    Entry, PageTable, PageTableConfig, PageTableError, PageTableGuard, PagingConstsTrait,
    PagingLevel, PteState, PteStateRef, page_size, pte_index,
};
use crate::{
    mm::{
        PageProperty, Vaddr,
        page_table::{PageTableNode, is_valid_range},
    },
    sync::RcuDrop,
    task::atomic_mode::InAtomicMode,
};

#[derive(Debug)]
pub(crate) struct Cursor<'rcu, C: PageTableConfig> {
    path: [Option<PageTableGuard<'rcu, C>>; MAX_NR_LEVELS],
    rcu_guard: &'rcu dyn InAtomicMode,
    level: PagingLevel,
    guard_level: PagingLevel,
    va: Vaddr,
    barrier_va: Range<Vaddr>,
    _phantom: PhantomData<&'rcu PageTable<C>>,
}

const MAX_NR_LEVELS: usize = 4;

#[must_use]
#[derive(Debug)]
pub(crate) enum PageTableFrag<C: PageTableConfig> {
    Mapped {
        va: Vaddr,
        item: RcuDrop<C::Item>,
    },
    StrayPageTable {
        pt: RcuDrop<PageTableNode<C>>,
        va: Vaddr,
        len: usize,
        num_frames: usize,
    },
}

impl<'rcu, C: PageTableConfig> Cursor<'rcu, C> {
    pub(in crate::mm) fn new(
        pt: &'rcu PageTable<C>,
        guard: &'rcu dyn InAtomicMode,
        va: &Range<Vaddr>,
        min_level: PagingLevel,
    ) -> Result<Self, PageTableError> {
        assert!((1..=C::NR_LEVELS).contains(&min_level));

        if !is_valid_range::<C>(va) {
            return Err(PageTableError::InvalidVaddrRange(va.start, va.end));
        }
        if !va.start.is_multiple_of(C::BASE_PAGE_SIZE) || !va.end.is_multiple_of(C::BASE_PAGE_SIZE)
        {
            return Err(PageTableError::UnalignedVaddr);
        }

        const { assert!(C::NR_LEVELS as usize <= MAX_NR_LEVELS) };

        Ok(locking::lock_range(pt, guard, va, min_level))
    }

    pub(crate) fn virt_addr(&self) -> Vaddr {
        self.va
    }

    pub(crate) fn query(&mut self) -> Result<PagesState<'rcu, C>, PageTableError> {
        if self.va >= self.barrier_va.end {
            return Err(PageTableError::InvalidVaddr(self.va));
        }

        let rcu_guard = self.rcu_guard;

        loop {
            let cur_entry = self.cur_entry();
            let item = match cur_entry.to_ref() {
                PteStateRef::PageTable(pt) => {
                    let guard = unsafe { pt.make_guard_unchecked(rcu_guard) };
                    self.push_level(guard);
                    continue;
                }
                PteStateRef::Absent => None,
                PteStateRef::Mapped(item) => Some(item),
            };

            return Ok((self.cur_va_range(), item));
        }
    }

    pub(crate) fn find_next(&mut self, len: usize) -> Option<Vaddr> {
        self.find_next_impl(len, false, false)
    }

    fn find_next_impl(
        &mut self,
        len: usize,
        find_unmap_subtree: bool,
        split_huge: bool,
    ) -> Option<Vaddr> {
        assert_eq!(len % C::BASE_PAGE_SIZE, 0);
        assert!(
            len <= self.barrier_va.end - self.va,
            "len exceeds remaining cursor range"
        );

        let end = self.va + len;
        debug_assert_eq!(end % C::BASE_PAGE_SIZE, 0);

        let rcu_guard = self.rcu_guard;

        while self.va < end {
            let cur_va = self.va;
            let cur_va_range = self.cur_va_range();
            let cur_entry_fits_range = cur_va == cur_va_range.start && cur_va_range.end <= end;

            let mut cur_entry = self.cur_entry();
            match cur_entry.to_ref() {
                PteStateRef::PageTable(pt) => {
                    if find_unmap_subtree
                        && cur_entry_fits_range
                        && (C::TOP_LEVEL_CAN_UNMAP || self.level != C::NR_LEVELS)
                    {
                        return Some(cur_va);
                    }

                    let pt_guard = unsafe { pt.make_guard_unchecked(rcu_guard) };
                    if pt_guard.nr_children() != 0 {
                        self.push_level(pt_guard);
                    } else {
                        let _ = ManuallyDrop::new(pt_guard);
                        self.move_forward();
                    }
                    continue;
                }
                PteStateRef::Absent => {
                    self.move_forward();
                    continue;
                }
                PteStateRef::Mapped(_) => {
                    if cur_entry_fits_range || !split_huge {
                        return Some(cur_va);
                    }

                    let split_child = cur_entry
                        .split_if_mapped_huge(rcu_guard)
                        .expect("the entry must be a huge page");
                    self.push_level(split_child);
                    continue;
                }
            }
        }

        if self.va > self.barrier_va.end {
            self.va = self.barrier_va.end;
        }

        None
    }

    pub(crate) fn jump(&mut self, va: Vaddr) -> Result<(), PageTableError> {
        assert!(va.is_multiple_of(C::BASE_PAGE_SIZE));
        if !self.barrier_va.contains(&va) {
            return Err(PageTableError::InvalidVaddr(va));
        }

        if self.va == self.barrier_va.end {
            while self.level < self.guard_level {
                self.pop_level();
            }
            self.va = va;
            return Ok(());
        }
        debug_assert!(self.barrier_va.contains(&self.va));

        loop {
            let node_size = page_size::<C>(self.level + 1);
            let node_start = self.va.align_down(node_size);
            if node_start <= va && va - node_start < node_size {
                self.va = va;
                return Ok(());
            }

            self.pop_level();
        }
    }

    fn move_forward(&mut self) {
        let next_va = self.cur_va_range().end;
        while self.level < self.guard_level && pte_index::<C>(next_va, self.level) == 0 {
            self.pop_level();
        }
        self.va = next_va;
    }

    fn pop_level(&mut self) {
        let taken = self.path[self.level as usize - 1]
            .take()
            .expect("popping a level without a lock");
        let _ = ManuallyDrop::new(taken);

        debug_assert!(self.level < self.guard_level);
        self.level += 1;
    }

    fn push_level(&mut self, child_pt: PageTableGuard<'rcu, C>) {
        self.level -= 1;
        debug_assert_eq!(self.level, child_pt.level());

        let old = self.path[self.level as usize - 1].replace(child_pt);
        debug_assert!(old.is_none());
    }

    fn cur_entry(&mut self) -> Entry<'_, 'rcu, C> {
        let node = self.path[self.level as usize - 1].as_mut().unwrap();
        node.entry(pte_index::<C>(self.va, self.level))
    }

    fn cur_va_range(&self) -> Range<Vaddr> {
        let entry_size = page_size::<C>(self.level);
        let entry_start = self.va.align_down(entry_size);
        entry_start..entry_start + entry_size
    }
}

impl<C: PageTableConfig> Drop for Cursor<'_, C> {
    fn drop(&mut self) {
        locking::unlock_range(self);
    }
}

pub(crate) type PagesState<'a, C> = (Range<Vaddr>, Option<<C as PageTableConfig>::ItemRef<'a>>);

#[derive(Debug)]
pub(crate) struct CursorMut<'rcu, C: PageTableConfig>(Cursor<'rcu, C>);

impl<'rcu, C: PageTableConfig> CursorMut<'rcu, C> {
    pub(super) fn new(
        pt: &'rcu PageTable<C>,
        guard: &'rcu dyn InAtomicMode,
        va: &Range<Vaddr>,
        min_level: PagingLevel,
    ) -> Result<Self, PageTableError> {
        Cursor::new(pt, guard, va, min_level).map(|inner| Self(inner))
    }

    pub(crate) fn find_next(&mut self, len: usize) -> Option<Vaddr> {
        self.0.find_next(len)
    }

    pub(crate) fn jump(&mut self, va: Vaddr) -> Result<(), PageTableError> {
        self.0.jump(va)
    }

    pub(crate) fn virt_addr(&self) -> Vaddr {
        self.0.virt_addr()
    }

    pub(crate) fn query(&mut self) -> Result<PagesState<'rcu, C>, PageTableError> {
        self.0.query()
    }

    pub(crate) unsafe fn map(&mut self, item: C::Item) {
        assert!(self.0.va < self.0.barrier_va.end);

        let (_, level, _) = C::item_raw_info(&item);
        assert!(
            level <= C::HIGHEST_TRANSLATION_LEVEL && level <= self.0.guard_level,
            "cursor level not suitable for mapping"
        );
        let size = page_size::<C>(level);
        assert_eq!(
            self.0.va % size,
            0,
            "cursor virtual address not aligned for mapping"
        );
        assert!(
            size <= self.0.barrier_va.end - self.0.va,
            "cursor virtual address out-of-bound for mapping"
        );

        let rcu_guard = self.0.rcu_guard;

        while self.0.level != level {
            if self.0.level < level {
                self.0.pop_level();
                continue;
            }
            let mut cur_entry = self.0.cur_entry();
            match cur_entry.to_ref() {
                PteStateRef::PageTable(pt) => {
                    let pt_guard = unsafe { pt.make_guard_unchecked(rcu_guard) };
                    self.0.push_level(pt_guard);
                }
                PteStateRef::Absent => {
                    let child_guard = cur_entry.alloc_if_none(rcu_guard).unwrap();
                    self.0.push_level(child_guard);
                }
                PteStateRef::Mapped(_) => {
                    let split_child = cur_entry.split_if_mapped_huge(rcu_guard).unwrap();
                    self.0.push_level(split_child);
                }
            }
        }

        if !matches!(self.0.cur_entry().to_ref(), PteStateRef::Absent) {
            panic!("mapping over an already mapped page");
        }

        let _ = self.replace_cur_entry(PteState::Mapped(RcuDrop::new(item)));

        self.0.move_forward();
    }

    pub(crate) unsafe fn take_next(&mut self, len: usize) -> Option<PageTableFrag<C>> {
        self.0.find_next_impl(len, true, true)?;

        let frag = self.replace_cur_entry(PteState::Absent);

        self.0.move_forward();

        frag
    }

    pub(crate) unsafe fn protect_next(
        &mut self,
        len: usize,
        op: &mut impl FnMut(&mut PageProperty),
    ) -> Option<Range<Vaddr>> {
        self.0.find_next_impl(len, false, true)?;

        self.0.cur_entry().protect(op);

        let protected_va = self.0.cur_va_range();

        self.0.move_forward();

        Some(protected_va)
    }

    fn replace_cur_entry(&mut self, new_child: PteState<C>) -> Option<PageTableFrag<C>> {
        let rcu_guard = self.0.rcu_guard;

        let va = self.0.va;
        let level = self.0.level;

        let old = self.0.cur_entry().replace(new_child);
        match old {
            PteState::Absent => None,
            PteState::Mapped(item) => Some(PageTableFrag::Mapped { va, item }),
            PteState::PageTable(pt) => {
                debug_assert_eq!(pt.level(), level - 1);

                if !C::TOP_LEVEL_CAN_UNMAP && level == C::NR_LEVELS {
                    let _ = ManuallyDrop::new(pt);
                    panic!("unmapping shared kernel page table nodes");
                }

                let locked_pt = unsafe { pt.borrow().make_guard_unchecked(rcu_guard) };
                let num_frames =
                    unsafe { locking::dfs_mark_stray_and_unlock(rcu_guard, locked_pt) };

                Some(PageTableFrag::StrayPageTable {
                    pt,
                    va,
                    len: page_size::<C>(self.0.level),
                    num_frames,
                })
            }
        }
    }
}
