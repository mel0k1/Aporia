use core::{marker::PhantomData, mem::ManuallyDrop, ops::Range, sync::atomic::Ordering};

use align_ext::AlignExt;

use super::Cursor;
use crate::{
    mm::{
        HasPaddr, Vaddr, nr_subpage_per_huge, paddr_to_vaddr,
        page_table::{
            PageTable, PageTableConfig, PageTableGuard, PageTableNodeRef, PagingConstsTrait,
            PagingLevel, PteScalar, PteStateRef, PteTrait, load_pte, page_size, pte_index,
        },
    },
    task::atomic_mode::InAtomicMode,
};

pub(super) fn lock_range<'rcu, C: PageTableConfig>(
    pt: &'rcu PageTable<C>,
    guard: &'rcu dyn InAtomicMode,
    va: &Range<Vaddr>,
    min_level: PagingLevel,
) -> Cursor<'rcu, C> {
    let mut subtree_root = loop {
        if let Some(subtree_root) = try_traverse_and_lock_subtree_root(pt, guard, va, min_level) {
            break subtree_root;
        }
    };

    let guard_level = subtree_root.level();
    let cur_node_va = va.start.align_down(page_size::<C>(guard_level + 1));
    dfs_acquire_lock(guard, &mut subtree_root, cur_node_va, va.clone());

    let mut path = core::array::from_fn(|_| None);
    path[guard_level as usize - 1] = Some(subtree_root);

    Cursor::<'rcu, C> {
        path,
        rcu_guard: guard,
        level: guard_level,
        guard_level,
        va: va.start,
        barrier_va: va.clone(),
        _phantom: PhantomData,
    }
}

pub(super) fn unlock_range<C: PageTableConfig>(cursor: &mut Cursor<'_, C>) {
    for i in (0..cursor.guard_level as usize - 1).rev() {
        if let Some(guard) = cursor.path[i].take() {
            let _ = ManuallyDrop::new(guard);
        }
    }
    let guard_node = cursor.path[cursor.guard_level as usize - 1].take().unwrap();
    let cur_node_va = cursor
        .barrier_va
        .start
        .align_down(page_size::<C>(cursor.guard_level + 1));

    unsafe {
        dfs_release_lock(
            cursor.rcu_guard,
            guard_node,
            cur_node_va,
            cursor.barrier_va.clone(),
        )
    };
}

fn try_traverse_and_lock_subtree_root<'rcu, C: PageTableConfig>(
    pt: &PageTable<C>,
    guard: &'rcu dyn InAtomicMode,
    va: &Range<Vaddr>,
    min_level: PagingLevel,
) -> Option<PageTableGuard<'rcu, C>> {
    let mut cur_node_guard: Option<PageTableGuard<C>> = None;
    let mut cur_pt_addr = pt.root.paddr();
    for cur_level in (1..=C::NR_LEVELS).rev() {
        let start_idx = pte_index::<C>(va.start, cur_level);
        let level_too_high = {
            let end_idx = pte_index::<C>(va.end - 1, cur_level);
            cur_level > min_level && start_idx == end_idx
        };
        if !level_too_high {
            break;
        }

        let cur_pt_ptr = paddr_to_vaddr(cur_pt_addr) as *mut C::E;
        let cur_pte = unsafe { load_pte(cur_pt_ptr.add(start_idx), Ordering::Acquire) };

        match cur_pte.to_repr(cur_level) {
            PteScalar::Mapped(_, _) => {
                break;
            }
            PteScalar::Absent => {}
            PteScalar::PageTable(child_pt_addr, _) => {
                cur_pt_addr = child_pt_addr;
                cur_node_guard = None;
                continue;
            }
        }

        let mut pt_guard = cur_node_guard.take().unwrap_or_else(|| {
            let node_ref = unsafe { PageTableNodeRef::<'rcu, C>::borrow_paddr(cur_pt_addr) };
            node_ref.lock(guard)
        });
        if *pt_guard.stray_mut() {
            return None;
        }

        let mut cur_entry = pt_guard.entry(start_idx);
        match cur_entry.to_ref() {
            PteStateRef::Mapped(_) => {
                break;
            }
            PteStateRef::Absent => {
                let allocated_guard = cur_entry.alloc_if_none(guard).unwrap();
                cur_pt_addr = allocated_guard.paddr();
                cur_node_guard = Some(allocated_guard);
            }
            PteStateRef::PageTable(pt) => {
                cur_pt_addr = pt.paddr();
                cur_node_guard = None;
            }
        }
    }

    let mut pt_guard = cur_node_guard.unwrap_or_else(|| {
        let node_ref = unsafe { PageTableNodeRef::<'rcu, C>::borrow_paddr(cur_pt_addr) };
        node_ref.lock(guard)
    });
    if *pt_guard.stray_mut() {
        return None;
    }

    Some(pt_guard)
}

fn dfs_acquire_lock<C: PageTableConfig>(
    guard: &dyn InAtomicMode,
    cur_node: &mut PageTableGuard<'_, C>,
    cur_node_va: Vaddr,
    va_range: Range<Vaddr>,
) {
    debug_assert!(!*cur_node.stray_mut());

    let cur_level = cur_node.level();
    if cur_level == 1 {
        return;
    }

    let idx_range = dfs_get_idx_range::<C>(cur_level, cur_node_va, &va_range);
    for i in idx_range {
        let child = cur_node.entry(i);
        match child.to_ref() {
            PteStateRef::PageTable(pt) => {
                let mut pt_guard = pt.lock(guard);
                let child_node_va = cur_node_va + i * page_size::<C>(cur_level);
                let child_node_va_end = child_node_va + page_size::<C>(cur_level);
                let va_start = va_range.start.max(child_node_va);
                let va_end = va_range.end.min(child_node_va_end);
                dfs_acquire_lock(guard, &mut pt_guard, child_node_va, va_start..va_end);
                let _ = ManuallyDrop::new(pt_guard);
            }
            PteStateRef::Absent | PteStateRef::Mapped(_) => {}
        }
    }
}

unsafe fn dfs_release_lock<'rcu, C: PageTableConfig>(
    guard: &'rcu dyn InAtomicMode,
    mut cur_node: PageTableGuard<'rcu, C>,
    cur_node_va: Vaddr,
    va_range: Range<Vaddr>,
) {
    let cur_level = cur_node.level();
    if cur_level == 1 {
        return;
    }

    let idx_range = dfs_get_idx_range::<C>(cur_level, cur_node_va, &va_range);
    for i in idx_range.rev() {
        let child = cur_node.entry(i);
        match child.to_ref() {
            PteStateRef::PageTable(pt) => {
                let child_node = unsafe { pt.make_guard_unchecked(guard) };
                let child_node_va = cur_node_va + i * page_size::<C>(cur_level);
                let child_node_va_end = child_node_va + page_size::<C>(cur_level);
                let va_start = va_range.start.max(child_node_va);
                let va_end = va_range.end.min(child_node_va_end);
                unsafe { dfs_release_lock(guard, child_node, child_node_va, va_start..va_end) };
            }
            PteStateRef::Absent | PteStateRef::Mapped(_) => {}
        }
    }
}

pub(super) unsafe fn dfs_mark_stray_and_unlock<C: PageTableConfig>(
    rcu_guard: &dyn InAtomicMode,
    mut sub_tree: PageTableGuard<C>,
) -> usize {
    *sub_tree.stray_mut() = true;

    if sub_tree.level() == 1 {
        return sub_tree.nr_children() as usize;
    }

    let mut num_frames = 0;

    for i in (0..nr_subpage_per_huge::<C>()).rev() {
        let child = sub_tree.entry(i);
        match child.to_ref() {
            PteStateRef::PageTable(pt) => {
                let locked_pt = unsafe { pt.make_guard_unchecked(rcu_guard) };
                num_frames += unsafe { dfs_mark_stray_and_unlock(rcu_guard, locked_pt) };
            }
            PteStateRef::Absent | PteStateRef::Mapped(_) => {}
        }
    }

    num_frames
}

fn dfs_get_idx_range<C: PagingConstsTrait>(
    cur_node_level: PagingLevel,
    cur_node_va: Vaddr,
    va_range: &Range<Vaddr>,
) -> Range<usize> {
    debug_assert!(va_range.start >= cur_node_va);
    debug_assert!(va_range.end <= cur_node_va.saturating_add(page_size::<C>(cur_node_level + 1)));

    let start_idx = (va_range.start - cur_node_va) / page_size::<C>(cur_node_level);
    let end_idx = (va_range.end - cur_node_va).div_ceil(page_size::<C>(cur_node_level));

    debug_assert!(start_idx < end_idx);
    debug_assert!(end_idx <= nr_subpage_per_huge::<C>());

    start_idx..end_idx
}
