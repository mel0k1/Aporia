use core::ops::Range;

use aster_block::BLOCK_SIZE;
use bitvec::{
    order::Lsb0,
    slice::BitSlice,
    view::{AsBits, AsMutBits},
};

use crate::prelude::*;

#[derive(Clone)]
pub(crate) struct IdBitmap {
    buf: Box<[u8]>,
    first_available_id: u16,
    len: u16,
}

impl IdBitmap {
    pub(crate) fn from_buf(buf: Box<[u8]>, len: u16) -> Self {
        assert!(len <= Self::capacity());
        let mut bitmap = Self {
            buf,
            first_available_id: 0,
            len,
        };

        bitmap.update_first_available_id(0);
        bitmap
    }

    #[expect(unused)]
    pub(crate) const fn len(&self) -> u16 {
        self.len
    }

    pub(crate) const fn capacity() -> u16 {
        BLOCK_SIZE as u16 * 8
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.buf
    }

    fn bit_slice(&self) -> &BitSlice<u8, Lsb0> {
        &self.buf.as_bits()[..self.len as usize]
    }

    fn bit_slice_mut(&mut self) -> &mut BitSlice<u8, Lsb0> {
        &mut self.buf.as_mut_bits()[..self.len as usize]
    }

    pub(crate) fn is_allocated(&self, id: u16) -> bool {
        self.bit_slice()[id as usize]
    }

    pub(crate) fn alloc(&mut self) -> Option<u16> {
        if self.first_available_id < self.len {
            let id = self.first_available_id;
            self.bit_slice_mut().set(id as usize, true);
            self.update_first_available_id(id + 1);
            Some(id)
        } else {
            None
        }
    }

    pub(crate) fn alloc_consecutive(&mut self, count: u16) -> Option<Range<u16>> {
        if count == 0 {
            return None;
        }

        let end = self.first_available_id.checked_add(count)?;
        if end > self.len {
            return None;
        }

        let allocated_range = {
            let bit_slice = self.bit_slice();
            let mut curr_range = self.first_available_id..self.first_available_id + 1;
            while curr_range.len() < count as usize && curr_range.end < self.len {
                if !bit_slice[curr_range.end as usize] {
                    curr_range.end += 1;
                } else {
                    curr_range = curr_range.end + 1..curr_range.end + 1;
                }
            }

            if curr_range.len() < count as usize {
                return None;
            }

            curr_range
        };

        let bit_slice_mut = self.bit_slice_mut();
        for id in allocated_range.clone() {
            bit_slice_mut.set(id as usize, true);
        }

        let bit_slice = self.bit_slice();
        if bit_slice[self.first_available_id as usize] {
            self.update_first_available_id(allocated_range.end);
        }

        Some(allocated_range)
    }

    pub(crate) fn free(&mut self, id: u16) {
        debug_assert!(self.bit_slice()[id as usize]);

        self.bit_slice_mut().set(id as usize, false);
        if id < self.first_available_id {
            self.first_available_id = id;
        }
    }

    pub(crate) fn free_consecutive(&mut self, range: Range<u16>) {
        if range.is_empty() {
            return;
        }

        let range_start = range.start;
        let bit_slice_mut = self.bit_slice_mut();
        for id in range {
            debug_assert!(bit_slice_mut[id as usize]);
            bit_slice_mut.set(id as usize, false);
        }

        if range_start < self.first_available_id {
            self.first_available_id = range_start
        }
    }

    fn update_first_available_id(&mut self, start: u16) {
        let len = self.len;
        let bit_slice = self
            .bit_slice()
            .get(start as usize..len as usize)
            .expect("start is guaranteed to be valid by the caller");
        self.first_available_id = bit_slice
            .first_zero()
            .map(|offset| start + offset as u16)
            .unwrap_or(len);
    }
}

impl Debug for IdBitmap {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("IdBitMap")
            .field("len", &self.len)
            .field("first_available_id", &self.first_available_id)
            .finish()
    }
}

#[cfg(ktest)]
mod test {
    use alloc::vec;

    use aster_block::BLOCK_SIZE;
    use ostd::prelude::ktest;

    use super::IdBitmap;

    #[ktest]
    fn bitmap_alloc_out_of_bounds() {
        let buf = vec![0; BLOCK_SIZE].into_boxed_slice();

        let capacity = BLOCK_SIZE as u16 * 8;
        let mut bitmap = IdBitmap::from_buf(buf, capacity);

        for _ in 0..capacity {
            assert!(bitmap.alloc().is_some());
        }

        assert!(bitmap.alloc_consecutive(1).is_none());
    }
}
