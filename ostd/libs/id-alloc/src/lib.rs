#![cfg_attr(not(test), no_std)]
#![deny(unsafe_code)]

use core::{fmt::Debug, ops::Range};

use bitvec::prelude::BitVec;

#[derive(Clone)]
pub struct IdAlloc {
    bitset: BitVec<u8>,
    first_available_id: usize,
}

impl IdAlloc {
    pub fn with_capacity(capacity: usize) -> Self {
        let mut bitset = BitVec::with_capacity(capacity);
        bitset.resize(capacity, false);
        Self {
            bitset,
            first_available_id: 0,
        }
    }

    pub fn alloc(&mut self) -> Option<usize> {
        if self.first_available_id < self.bitset.len() {
            let id = self.first_available_id;
            self.bitset.set(id, true);
            self.update_first_available_id(id + 1);
            Some(id)
        } else {
            None
        }
    }

    pub fn alloc_consecutive(&mut self, count: usize) -> Option<Range<usize>> {
        if count == 0 {
            return None;
        }

        let end = self.first_available_id.checked_add(count)?;
        if end > self.bitset.len() {
            return None;
        }

        let allocated_range = {
            let mut curr_range = self.first_available_id..self.first_available_id + 1;
            while curr_range.len() < count && curr_range.end < self.bitset.len() {
                if !self.is_allocated(curr_range.end) {
                    curr_range.end += 1;
                } else {
                    curr_range = curr_range.end + 1..curr_range.end + 1;
                }
            }

            if curr_range.len() < count {
                return None;
            }

            curr_range
        };

        for id in allocated_range.clone() {
            self.bitset.set(id, true);
        }

        if self.is_allocated(self.first_available_id) {
            self.update_first_available_id(allocated_range.end);
        }

        Some(allocated_range)
    }

    pub fn free_consecutive(&mut self, range: Range<usize>) {
        if range.is_empty() {
            return;
        }

        let range_start = range.start;
        for id in range {
            debug_assert!(self.is_allocated(id));
            self.bitset.set(id, false);
        }

        if range_start < self.first_available_id {
            self.first_available_id = range_start
        }
    }

    pub fn free(&mut self, id: usize) {
        debug_assert!(self.is_allocated(id));

        self.bitset.set(id, false);
        if id < self.first_available_id {
            self.first_available_id = id;
        }
    }

    pub fn alloc_specific(&mut self, id: usize) -> Option<usize> {
        if self.bitset[id] {
            return None;
        }
        self.bitset.set(id, true);
        if id == self.first_available_id {
            self.update_first_available_id(id + 1);
        }
        Some(id)
    }

    pub fn is_allocated(&self, id: usize) -> bool {
        self.bitset[id]
    }

    fn update_first_available_id(&mut self, start: usize) {
        let len = self.bitset.len();
        let bit_slice = self
            .bitset
            .get(start..len)
            .expect("start is guaranteed to be valid by the caller");
        self.first_available_id = bit_slice
            .first_zero()
            .map(|offset| start + offset)
            .unwrap_or(len);
    }
}

impl Debug for IdAlloc {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("IdAlloc")
            .field("len", &self.bitset.len())
            .field("first_available_id", &self.first_available_id)
            .finish()
    }
}

#[cfg(test)]
mod test {
    use super::IdAlloc;

    #[test]
    fn bitmap_alloc_out_of_bounds() {
        let capacity = 16;
        let mut bitmap = IdAlloc::with_capacity(capacity);

        for _ in 0..capacity {
            assert!(bitmap.alloc().is_some());
        }

        assert!(bitmap.alloc_consecutive(1).is_none());
    }
}
