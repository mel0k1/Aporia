mod entry;
mod pte_state;

use core::{
    cell::SyncUnsafeCell,
    marker::PhantomData,
    ops::Deref,
    sync::atomic::{AtomicU8, Ordering},
};

pub(in crate::mm) use self::{
    entry::Entry,
    pte_state::{PteState, PteStateRef},
};
use super::{PageTableConfig, PteTrait, nr_subpage_per_huge};
use crate::{
    mm::{
        FrameAllocOptions, HasPaddr, Infallible, PagingConstsTrait, PagingLevel, VmReader,
        frame::{Frame, FrameRef, meta::AnyFrameMeta},
        paddr_to_vaddr,
        page_table::{PteScalar, load_pte, store_pte},
    },
    task::atomic_mode::InAtomicMode,
};

pub(crate) type PageTableNode<C> = Frame<PageTablePageMeta<C>>;

impl<C: PageTableConfig> PageTableNode<C> {
    pub(super) fn level(&self) -> PagingLevel {
        self.meta().level
    }

    pub(super) fn alloc(level: PagingLevel) -> Self {
        let meta = PageTablePageMeta::new(level);
        FrameAllocOptions::new()
            .zeroed(true)
            .alloc_frame_with(meta)
            .expect("Failed to allocate a page table node")
    }

    pub(super) unsafe fn activate(&self) {
        use crate::arch::mm::{activate_page_table, current_page_table_paddr};

        assert_eq!(self.level(), C::NR_LEVELS);

        let last_activated_paddr = current_page_table_paddr();
        if last_activated_paddr == self.paddr() {
            return;
        }

        unsafe { activate_page_table(self.clone().into_raw()) };

        drop(unsafe { Self::from_raw(last_activated_paddr) });
    }

    pub(super) unsafe fn first_activate(&self) {
        use crate::arch::mm::activate_page_table;

        unsafe { activate_page_table(self.clone().into_raw()) };
    }
}

pub(super) type PageTableNodeRef<'a, C> = FrameRef<'a, PageTablePageMeta<C>>;

impl<'a, C: PageTableConfig> PageTableNodeRef<'a, C> {
    pub(super) fn lock<'rcu>(self, _guard: &'rcu dyn InAtomicMode) -> PageTableGuard<'rcu, C>
    where
        'a: 'rcu,
    {
        while self
            .meta()
            .lock
            .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }

        PageTableGuard::<'rcu, C> { inner: self }
    }

    pub(super) unsafe fn make_guard_unchecked<'rcu>(
        self,
        _guard: &'rcu dyn InAtomicMode,
    ) -> PageTableGuard<'rcu, C>
    where
        'a: 'rcu,
    {
        PageTableGuard { inner: self }
    }
}

#[derive(Debug)]
pub(super) struct PageTableGuard<'rcu, C: PageTableConfig> {
    inner: PageTableNodeRef<'rcu, C>,
}

impl<'rcu, C: PageTableConfig> PageTableGuard<'rcu, C> {
    pub(super) fn entry(&mut self, idx: usize) -> Entry<'_, 'rcu, C> {
        assert!(idx < nr_subpage_per_huge::<C>());
        unsafe { Entry::new_at(self, idx) }
    }

    pub(super) fn nr_children(&self) -> u16 {
        unsafe { *self.meta().nr_children.get() }
    }

    pub(super) fn stray_mut(&mut self) -> &mut bool {
        unsafe { &mut *self.meta().stray.get() }
    }

    pub(super) unsafe fn read_pte(&self, idx: usize) -> C::E {
        debug_assert!(idx < nr_subpage_per_huge::<C>());
        let ptr = paddr_to_vaddr(self.paddr()) as *mut C::E;
        unsafe { load_pte(ptr.add(idx), Ordering::Relaxed) }
    }

    pub(super) unsafe fn write_pte(&mut self, idx: usize, pte: C::E) {
        debug_assert!(idx < nr_subpage_per_huge::<C>());
        let ptr = paddr_to_vaddr(self.paddr()) as *mut C::E;
        unsafe { store_pte(ptr.add(idx), pte, Ordering::Release) }
    }

    fn nr_children_mut(&mut self) -> &mut u16 {
        unsafe { &mut *self.meta().nr_children.get() }
    }
}

impl<'rcu, C: PageTableConfig> Deref for PageTableGuard<'rcu, C> {
    type Target = PageTableNodeRef<'rcu, C>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<C: PageTableConfig> Drop for PageTableGuard<'_, C> {
    fn drop(&mut self) {
        self.inner.meta().lock.store(0, Ordering::Release);
    }
}

#[derive(Debug)]
pub(crate) struct PageTablePageMeta<C: PageTableConfig> {
    nr_children: SyncUnsafeCell<u16>,
    stray: SyncUnsafeCell<bool>,
    level: PagingLevel,
    lock: AtomicU8,
    _phantom: PhantomData<C>,
}

impl<C: PageTableConfig> PageTablePageMeta<C> {
    fn new(level: PagingLevel) -> Self {
        Self {
            nr_children: SyncUnsafeCell::new(0),
            stray: SyncUnsafeCell::new(false),
            level,
            lock: AtomicU8::new(0),
            _phantom: PhantomData,
        }
    }
}

unsafe impl<C: PageTableConfig> AnyFrameMeta for PageTablePageMeta<C> {
    fn on_drop(&mut self, reader: &mut VmReader<Infallible>) {
        let nr_children = self.nr_children.get_mut();
        if *nr_children == 0 {
            return;
        }

        let level = self.level;
        let range = if level == C::NR_LEVELS {
            C::TOP_LEVEL_INDEX_RANGE.clone()
        } else {
            0..nr_subpage_per_huge::<C>()
        };

        reader.skip(range.start * size_of::<C::E>());
        for _ in range {
            let pte = reader.read_once::<C::E>().unwrap();
            match pte.to_repr(level) {
                PteScalar::PageTable(child_pt_addr, _) => {
                    drop(unsafe { PageTableNode::<C>::from_raw(child_pt_addr) });
                }
                PteScalar::Mapped(pa, prop) => {
                    drop(unsafe { C::item_from_raw(pa, level, prop) });
                }
                PteScalar::Absent => {}
            }
        }
    }
}
