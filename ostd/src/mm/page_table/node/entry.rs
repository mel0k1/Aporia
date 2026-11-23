use super::{PageTableGuard, PageTableNode, PteState, PteStateRef, PteTrait};
use crate::{
    mm::{
        HasPaddr, nr_subpage_per_huge,
        page_prop::PageProperty,
        page_size,
        page_table::{PageTableConfig, PageTableNodeRef, PteScalar},
    },
    panic::PanicGuard,
    sync::RcuDrop,
    task::atomic_mode::InAtomicMode,
};

pub(in crate::mm) struct Entry<'a, 'rcu, C: PageTableConfig> {
    pte: C::E,
    idx: usize,
    node: &'a mut PageTableGuard<'rcu, C>,
}

impl<'a, 'rcu, C: PageTableConfig> Entry<'a, 'rcu, C> {
    pub(in crate::mm) fn to_ref(&self) -> PteStateRef<'rcu, C> {
        unsafe { PteStateRef::from_pte(&self.pte, self.node.level()) }
    }

    pub(in crate::mm) fn protect(&mut self, op: &mut impl FnMut(&mut PageProperty)) {
        let level = self.node.level();
        let PteScalar::Mapped(pa, prop) = self.pte.to_repr(level) else {
            return;
        };

        let mut new_prop = prop;
        op(&mut new_prop);

        if prop == new_prop {
            return;
        }

        self.pte = C::E::from_repr(&PteScalar::Mapped(pa, new_prop), level);

        unsafe { self.node.write_pte(self.idx, self.pte) };
    }

    pub(in crate::mm) fn replace(&mut self, new_child: PteState<C>) -> PteState<C> {
        match &new_child {
            PteState::PageTable(node) => {
                assert_eq!(node.level(), self.node.level() - 1);
            }
            PteState::Mapped(item) => {
                assert_eq!(C::item_raw_info(&**item).1, self.node.level());
            }
            PteState::Absent => {}
        }

        let old_child = unsafe { PteState::from_pte(self.pte, self.node.level()) };

        if old_child.is_absent() && !new_child.is_absent() {
            *self.node.nr_children_mut() += 1;
        } else if !old_child.is_absent() && new_child.is_absent() {
            *self.node.nr_children_mut() -= 1;
        }

        self.pte = new_child.into_pte();

        unsafe { self.node.write_pte(self.idx, self.pte) };

        old_child
    }

    pub(in crate::mm::page_table) fn alloc_if_none(
        &mut self,
        guard: &'rcu dyn InAtomicMode,
    ) -> Option<PageTableGuard<'rcu, C>> {
        if !matches!(self.to_ref(), PteStateRef::Absent) || self.node.level() == 1 {
            return None;
        }

        let level = self.node.level();
        let new_page = RcuDrop::new(PageTableNode::<C>::alloc(level - 1));

        let paddr = new_page.paddr();
        let pt_ref = unsafe { PageTableNodeRef::borrow_paddr(paddr) };

        let pt_lock_guard = pt_ref.lock(guard);

        self.pte = PteState::PageTable(new_page).into_pte();

        unsafe { self.node.write_pte(self.idx, self.pte) };

        *self.node.nr_children_mut() += 1;

        Some(pt_lock_guard)
    }

    pub(in crate::mm::page_table) fn split_if_mapped_huge(
        &mut self,
        guard: &'rcu dyn InAtomicMode,
    ) -> Option<PageTableGuard<'rcu, C>> {
        let level = self.node.level();
        let PteScalar::Mapped(pa, prop) = self.pte.to_repr(level) else {
            return None;
        };

        let new_page = RcuDrop::new(PageTableNode::<C>::alloc(level - 1));

        let paddr = new_page.paddr();
        let pt_ref = unsafe { PageTableNodeRef::borrow_paddr(paddr) };

        let mut pt_lock_guard = pt_ref.lock(guard);

        let panic_guard = PanicGuard::new();

        for i in 0..nr_subpage_per_huge::<C>() {
            let small_pa = pa + i * page_size::<C>(level - 1);
            let mut entry = pt_lock_guard.entry(i);
            let small_item = unsafe { C::item_from_raw(small_pa, level - 1, prop) };
            let old = entry.replace(PteState::Mapped(RcuDrop::new(small_item)));
            debug_assert!(old.is_absent());
        }

        self.pte = PteState::PageTable(new_page).into_pte();

        unsafe { self.node.write_pte(self.idx, self.pte) };

        panic_guard.forget();

        Some(pt_lock_guard)
    }

    pub(super) unsafe fn new_at(guard: &'a mut PageTableGuard<'rcu, C>, idx: usize) -> Self {
        let pte = unsafe { guard.read_pte(idx) };
        Self {
            pte,
            idx,
            node: guard,
        }
    }
}
