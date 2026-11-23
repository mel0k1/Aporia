use alloc::vec::Vec;
use core::ops::Range;

#[derive(Clone, Debug)]
pub struct SlotVec<T> {
    slots: Vec<Option<T>>,
    num_occupied: usize,
}

impl<T> SlotVec<T> {
    pub const fn new() -> Self {
        Self {
            slots: Vec::new(),
            num_occupied: 0,
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            slots: Vec::with_capacity(capacity),
            num_occupied: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.num_occupied == 0
    }

    pub fn len(&self) -> usize {
        self.num_occupied
    }

    pub fn slots_len(&self) -> usize {
        self.slots.len()
    }

    pub fn get(&self, idx: usize) -> Option<&T> {
        self.slots.get(idx)?.as_ref()
    }

    pub fn get_mut(&mut self, idx: usize) -> Option<&mut T> {
        self.slots.get_mut(idx)?.as_mut()
    }

    pub fn put(&mut self, entry: T) -> usize {
        match self.try_put_in(entry, 0..usize::MAX) {
            Ok(index) => index,
            Err(_) => unreachable!("a `SlotVec` cannot occupy every index below `usize::MAX`"),
        }
    }

    pub fn try_put_in(&mut self, entry: T, range: Range<usize>) -> Result<usize, T> {
        let append_idx = self.slots.len().max(range.start);
        let idx = if self.num_occupied == self.slots.len() {
            append_idx
        } else {
            (range.start..range.end.min(self.slots.len()))
                .find(|&idx| self.slots[idx].is_none())
                .unwrap_or(append_idx)
        };
        if idx >= range.end {
            return Err(entry);
        }

        self.put_at(idx, entry);
        Ok(idx)
    }

    pub fn put_at(&mut self, idx: usize, item: T) -> Option<T> {
        if idx >= self.slots.len() {
            self.slots.resize_with(idx + 1, Default::default);
        }
        let mut sub_item = Some(item);
        core::mem::swap(&mut sub_item, &mut self.slots[idx]);
        if sub_item.is_none() {
            self.num_occupied += 1;
        }
        sub_item
    }

    pub fn remove(&mut self, idx: usize) -> Option<T> {
        if idx >= self.slots.len() {
            return None;
        }
        let mut del_item = None;
        core::mem::swap(&mut del_item, &mut self.slots[idx]);
        if del_item.is_some() {
            debug_assert!(self.num_occupied > 0);
            self.num_occupied -= 1;
        }
        del_item
    }

    pub fn idxes_and_items(&self) -> impl Iterator<Item = (usize, &'_ T)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(idx, slot)| slot.as_ref().map(|val| (idx, val)))
    }

    pub fn iter(&self) -> impl Iterator<Item = &'_ T> {
        self.slots.iter().filter_map(|x| x.as_ref())
    }
}

impl Default for SlotVec<()> {
    fn default() -> Self {
        Self::new()
    }
}
