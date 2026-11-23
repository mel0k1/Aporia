use core::ops::RangeInclusive;

use super::{RecordKey, RecordValue};
use crate::{prelude::*, util::BitMap};

#[derive(Debug)]
pub(crate) struct RangeQueryCtx<K, V> {
    start: K,
    num_values: usize,
    complete_table: BitMap,
    min_uncompleted: usize,
    res: Vec<(K, V)>,
}

impl<K: RecordKey<K>, V: RecordValue> RangeQueryCtx<K, V> {
    pub(crate) fn new(start: K, num_values: usize) -> Self {
        Self {
            start,
            num_values,
            complete_table: BitMap::repeat(false, num_values),
            min_uncompleted: 0,
            res: Vec::with_capacity(num_values),
        }
    }

    pub(crate) fn range_uncompleted(&self) -> Option<RangeInclusive<K>> {
        if self.is_completed() {
            return None;
        }
        debug_assert!(self.min_uncompleted < self.num_values);

        let first_uncompleted = self.start + self.min_uncompleted;
        let last_uncompleted = self.start + self.complete_table.last_zero()?;
        Some(first_uncompleted..=last_uncompleted)
    }

    pub(crate) fn contains_uncompleted(&self, key: &K) -> bool {
        let nth = *key - self.start;
        nth < self.num_values && !self.complete_table[nth]
    }

    pub(crate) fn is_completed(&self) -> bool {
        self.min_uncompleted == self.num_values
    }

    pub(crate) fn complete(&mut self, key: K, value: V) {
        let nth = key - self.start;
        if self.complete_table[nth] {
            return;
        }

        self.res.push((key, value));
        self.complete_table.set(nth, true);
        self.update_min_uncompleted(nth);
    }

    pub(crate) fn mark_completed(&mut self, key: K) {
        let nth = key - self.start;

        self.complete_table.set(nth, true);
        self.update_min_uncompleted(nth);
    }

    pub(crate) fn into_results(self) -> Vec<(K, V)> {
        debug_assert!(self.is_completed());
        self.res
    }

    fn update_min_uncompleted(&mut self, completed_nth: usize) {
        if self.min_uncompleted == completed_nth {
            if let Some(next_uncompleted) = self.complete_table.first_zero(completed_nth) {
                self.min_uncompleted = next_uncompleted;
            } else {
                self.min_uncompleted = self.num_values;
            }
        }
    }
}
