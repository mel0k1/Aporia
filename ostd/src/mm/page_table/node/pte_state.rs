use core::mem::ManuallyDrop;

use ostd_pod::FromZeros;

use super::{PageTableNode, PageTableNodeRef, PteTrait};
use crate::{
    mm::{
        HasPaddr, PageTableFlags, PagingLevel,
        page_table::{PageTableConfig, PteScalar},
    },
    sync::RcuDrop,
};

#[derive(Debug)]
pub(in crate::mm) enum PteState<C: PageTableConfig> {
    PageTable(RcuDrop<PageTableNode<C>>),
    Mapped(RcuDrop<C::Item>),
    Absent,
}

impl<C: PageTableConfig> PteState<C> {
    pub(in crate::mm) fn is_absent(&self) -> bool {
        matches!(self, PteState::Absent)
    }

    pub(super) fn into_pte(self) -> C::E {
        match self {
            PteState::PageTable(node) => {
                let paddr = node.paddr();
                let level = node.level();
                let _ = ManuallyDrop::new(node);
                C::E::from_repr(&PteScalar::PageTable(paddr, PageTableFlags::empty()), level)
            }
            PteState::Mapped(item) => {
                let (item, panic_guard) = unsafe { RcuDrop::into_inner(item) };
                let (paddr, level, prop) = C::item_into_raw(item);
                panic_guard.forget();
                C::E::from_repr(&PteScalar::Mapped(paddr, prop), level)
            }
            PteState::Absent => C::E::new_zeroed(),
        }
    }

    pub(super) unsafe fn from_pte(pte: C::E, level: PagingLevel) -> Self {
        let repr = pte.to_repr(level);

        match repr {
            PteScalar::Absent => PteState::Absent,
            PteScalar::PageTable(paddr, _) => {
                let node = unsafe { PageTableNode::from_raw(paddr) };
                debug_assert_eq!(node.level(), level - 1);
                PteState::PageTable(RcuDrop::new(node))
            }
            PteScalar::Mapped(paddr, prop) => {
                let item = unsafe { C::item_from_raw(paddr, level, prop) };
                PteState::Mapped(RcuDrop::new(item))
            }
        }
    }
}

#[derive(Debug)]
pub(in crate::mm) enum PteStateRef<'a, C: PageTableConfig> {
    PageTable(PageTableNodeRef<'a, C>),
    Mapped(C::ItemRef<'a>),
    Absent,
}

impl<C: PageTableConfig> PteStateRef<'_, C> {
    pub(super) unsafe fn from_pte(pte: &C::E, level: PagingLevel) -> Self {
        let repr = pte.to_repr(level);

        match repr {
            PteScalar::Absent => PteStateRef::Absent,
            PteScalar::PageTable(paddr, _) => {
                let node = unsafe { PageTableNodeRef::borrow_paddr(paddr) };
                debug_assert_eq!(node.level(), level - 1);
                PteStateRef::PageTable(node)
            }
            PteScalar::Mapped(paddr, prop) => {
                let item = unsafe { C::item_ref_from_raw(paddr, level, prop) };
                PteStateRef::Mapped(item)
            }
        }
    }
}
