use alloc::sync::Arc;
use core::ops::{Deref, DerefMut};

use ostd::{
    sync::{SpinGuardian, SpinLockGuard, non_null::NonNullPtr},
    task::atomic_mode::{AsAtomicModeGuard, InAtomicMode},
    util::Either,
};

use crate::{
    SLOT_SIZE, XArray, XLockGuard,
    entry::NodeEntryRef,
    mark::{NoneMark, PRESENT_MARK, XMark},
    node::{Height, XNode},
};

#[derive(Default)]
enum CursorState<'a, P>
where
    P: NonNullPtr + Send + Sync,
{
    #[default]
    Inactive,
    AtNode {
        node: NodeEntryRef<'a, P>,
        operation_offset: u8,
    },
}

impl<'a, P: NonNullPtr + Send + Sync> CursorState<'a, P> {
    fn move_to(&mut self, node: NodeEntryRef<'a, P>, index: u64) {
        let operation_offset = node.entry_offset(index);
        *self = Self::AtNode {
            node,
            operation_offset,
        };
    }

    fn as_node(&self) -> Option<(&NodeEntryRef<'a, P>, u8)> {
        match self {
            Self::AtNode {
                node,
                operation_offset,
            } => Some((node, *operation_offset)),
            Self::Inactive => None,
        }
    }

    fn into_node(self) -> Option<(NodeEntryRef<'a, P>, u8)> {
        match self {
            Self::AtNode {
                node,
                operation_offset,
            } => Some((node, operation_offset)),
            Self::Inactive => None,
        }
    }

    fn is_at_node(&self) -> bool {
        match self {
            Self::AtNode { .. } => true,
            Self::Inactive => false,
        }
    }

    fn is_at_leaf(&self) -> bool {
        match self {
            Self::AtNode { node, .. } => node.is_leaf(),
            Self::Inactive => false,
        }
    }
}

pub struct Cursor<'a, P, M = NoneMark>
where
    P: NonNullPtr + Send + Sync,
{
    xa: &'a XArray<P, M>,
    index: u64,
    guard: &'a dyn InAtomicMode,
    state: CursorState<'a, P>,
}

impl<'a, P: NonNullPtr + Send + Sync, M> Cursor<'a, P, M> {
    pub(super) fn new<G: AsAtomicModeGuard>(
        xa: &'a XArray<P, M>,
        guard: &'a G,
        index: u64,
    ) -> Self {
        Self {
            xa,
            index,
            guard: guard.as_atomic_mode_guard(),
            state: CursorState::Inactive,
        }
    }

    fn traverse_to_target(&mut self) {
        if self.state.is_at_node() {
            return;
        }

        let Some(head) = self.xa.head.read_with(self.guard) else {
            return;
        };

        let max_index = head.height().max_index();
        if max_index < self.index {
            return;
        }

        self.state.move_to(head, self.index);
        self.continue_traverse_to_target();
    }

    fn continue_traverse_to_target(&mut self) {
        while !self.state.is_at_leaf() {
            let (current_node, operation_offset) =
                core::mem::take(&mut self.state).into_node().unwrap();

            let Some(next_node) = current_node
                .deref_target()
                .entry_with(self.guard, operation_offset)
                .map(|operated_entry| operated_entry.left().unwrap())
            else {
                self.reset();
                return;
            };

            self.state.move_to(next_node, self.index);
        }
    }

    fn find_marked(&mut self, mark: usize) -> Option<u64> {
        let mut index = self.index.checked_add(1)?;
        let (mut current_node, mut operation_offset) =
            if let Some((node, offset)) = core::mem::take(&mut self.state).into_node() {
                (node, offset + 1)
            } else if let Some(node) = self.xa.head.read_with(self.guard)
                && index <= node.height().max_index()
            {
                let offset = node.entry_offset(index);
                (node, offset)
            } else {
                self.reset();
                return None;
            };

        loop {
            if operation_offset == SLOT_SIZE as u8 {
                let Some(parent_node) = current_node.deref_target().parent(self.guard) else {
                    self.reset();
                    return None;
                };

                operation_offset = current_node.offset_in_parent() + 1;
                current_node = parent_node;
                continue;
            }

            let new_operation_offset = current_node
                .next_marked(operation_offset, mark)
                .unwrap_or(SLOT_SIZE as u8);
            let gap = (new_operation_offset - operation_offset) as u64;
            if gap != 0 {
                let index_step = current_node.height().index_step();
                let Some(new_index) = (index & !(index_step - 1)).checked_add(gap * index_step)
                else {
                    self.reset();
                    return None;
                };

                index = new_index;
                operation_offset = new_operation_offset;
                if new_operation_offset == SLOT_SIZE as u8 {
                    continue;
                }
            }

            let Some(child_node) = current_node
                .deref_target()
                .entry_with(self.guard, operation_offset)
            else {
                continue;
            };

            if !current_node.is_leaf() {
                current_node = child_node.left().unwrap();
                operation_offset = current_node.entry_offset(index);
                continue;
            }

            self.index = index;
            self.state = CursorState::AtNode {
                node: current_node,
                operation_offset,
            };
            return Some(index);
        }
    }

    pub fn load(&mut self) -> Option<P::Ref<'a>> {
        self.traverse_to_target();
        let (node, operation_offset) = self.state.as_node()?;
        node.deref_target()
            .entry_with(self.guard, operation_offset)
            .and_then(|item_entry| item_entry.right())
    }

    pub fn index(&self) -> u64 {
        self.index
    }

    pub fn reset_to(&mut self, index: u64) {
        self.reset();
        self.index = index;
    }

    pub fn reset(&mut self) {
        self.state = CursorState::Inactive;
    }

    pub fn next(&mut self) {
        self.index = self.index.checked_add(1).unwrap();

        if !self.state.is_at_node() {
            return;
        }

        let (mut current_node, mut operation_offset) =
            core::mem::take(&mut self.state).into_node().unwrap();

        operation_offset += 1;
        while operation_offset == SLOT_SIZE as u8 {
            let Some(parent_node) = current_node.deref_target().parent(self.guard) else {
                self.reset();
                return;
            };

            operation_offset = current_node.offset_in_parent() + 1;
            current_node = parent_node;
        }

        self.state.move_to(current_node, self.index);
        self.continue_traverse_to_target();
    }

    pub fn next_present(&mut self) -> Option<u64> {
        self.find_marked(PRESENT_MARK)
    }
}

impl<P: NonNullPtr + Send + Sync, M: Into<XMark>> Cursor<'_, P, M> {
    pub fn is_marked(&mut self, mark: M) -> bool {
        self.traverse_to_target();
        self.state
            .as_node()
            .map(|(node, off)| node.is_marked(off, mark.into().index()))
            .unwrap_or(false)
    }

    pub fn next_marked(&mut self, mark: M) -> Option<u64> {
        self.find_marked(mark.into().index())
    }
}

pub struct CursorMut<'a, P, M>(Cursor<'a, P, M>)
where
    P: NonNullPtr + Send + Sync;

impl<'a, P: NonNullPtr + Send + Sync, M> CursorMut<'a, P, M> {
    pub(super) fn new<G: SpinGuardian>(
        xa: &'a XArray<P, M>,
        guard: &'a SpinLockGuard<'a, (), G>,
        index: u64,
    ) -> Self {
        Self(Cursor::new(xa, guard, index))
    }

    fn lock_guard(&self) -> XLockGuard<'_> {
        XLockGuard(self.guard)
    }

    fn reserve(&self, index: u64) {
        if self.xa.head.read_with(self.guard).is_none() {
            let height = Height::from_index(index);
            let new_head = Arc::new(XNode::new_root(height));
            self.xa.head.update(Some(new_head));
            return;
        };

        loop {
            let head = self.xa.head.read_with(self.guard).unwrap();
            let height = head.height();
            if height.max_index() >= index {
                return;
            }

            let new_head = Arc::new(XNode::new_root(height.go_root()));
            new_head.set_entry(self.lock_guard(), 0, Some(Either::Left(head.clone())));

            self.xa.head.update(Some(new_head));
        }
    }

    fn expand_and_traverse_to_target(&mut self) {
        if self.state.is_at_node() {
            return;
        }

        let head = {
            self.reserve(self.index);
            self.xa.head.read_with(self.guard).unwrap()
        };

        self.0.state.move_to(head, self.0.index);
        self.continue_traverse_to_target_mut();
    }

    fn continue_traverse_to_target_mut(&mut self) {
        while !self.state.is_at_leaf() {
            let (current_node, operation_offset) =
                core::mem::take(&mut self.state).into_node().unwrap();

            if current_node
                .entry_with(self.guard, operation_offset)
                .is_none()
            {
                let new_node = XNode::new(current_node.height().go_leaf(), operation_offset);
                let new_entry = Either::Left(Arc::new(new_node));
                current_node.set_entry(self.lock_guard(), operation_offset, Some(new_entry));
            }

            let next_node = current_node
                .deref_target()
                .entry_with(self.guard, operation_offset)
                .unwrap()
                .left()
                .unwrap();

            self.0.state.move_to(next_node, self.0.index);
        }
    }

    pub fn store(&mut self, item: P) {
        self.expand_and_traverse_to_target();
        let (node, operation_offset) = self.state.as_node().unwrap();
        node.set_entry(
            self.lock_guard(),
            operation_offset,
            Some(Either::Right(item)),
        );
    }

    pub fn remove(&mut self) -> Option<P::Ref<'a>> {
        self.traverse_to_target();
        let (node, off) = self.state.as_node()?;

        let res = node
            .deref_target()
            .entry_with(self.guard, off)
            .and_then(|entry| entry.right());
        node.set_entry(self.lock_guard(), off, None);

        if !node.is_empty() {
            return res;
        }

        let (mut current, _) = core::mem::take(&mut self.state).into_node().unwrap();

        loop {
            let Some(parent) = current.deref_target().parent(self.guard) else {
                self.xa.head.update(None);
                break;
            };

            parent.set_entry(self.lock_guard(), current.offset_in_parent(), None);

            if !parent.is_empty() {
                break;
            }
            current = parent;
        }

        res
    }
}

#[derive(Debug)]
pub struct SetMarkError;

impl<P: NonNullPtr + Send + Sync, M: Into<XMark>> CursorMut<'_, P, M> {
    pub fn set_mark(&mut self, mark: M) -> Result<(), SetMarkError> {
        self.traverse_to_target();
        self.state
            .as_node()
            .filter(|(node, off)| {
                node.entry_with(self.guard, *off)
                    .is_some_and(|entry| entry.is_right())
            })
            .map(|(node, off)| {
                let mark_index = mark.into().index();
                node.set_mark(self.lock_guard(), off, mark_index);
            })
            .ok_or(SetMarkError)
    }

    pub fn unset_mark(&mut self, mark: M) -> Result<(), SetMarkError> {
        self.traverse_to_target();
        self.state
            .as_node()
            .filter(|(node, off)| {
                node.entry_with(self.guard, *off)
                    .is_some_and(|entry| entry.is_right())
            })
            .map(|(node, off)| {
                let mark_index = mark.into().index();
                node.unset_mark(self.lock_guard(), off, mark_index);
            })
            .ok_or(SetMarkError)
    }
}

impl<'a, P: NonNullPtr + Send + Sync, M> Deref for CursorMut<'a, P, M> {
    type Target = Cursor<'a, P, M>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<P: NonNullPtr + Send + Sync, M> DerefMut for CursorMut<'_, P, M> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
