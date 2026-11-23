use alloc::collections::btree_map::{BTreeMap, Cursor, CursorMut};
use core::ops::Range;

pub(crate) trait Interval<K: Clone> {
    fn range(&self) -> Range<K>;
}

#[derive(Debug)]
pub(crate) struct IntervalSet<K, V>
where
    K: Clone + Ord,
    V: Interval<K>,
{
    btree: BTreeMap<K, V>,
}

impl<K, V> Default for IntervalSet<K, V>
where
    K: Clone + Ord,
    V: Interval<K>,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<K, V> IntervalSet<K, V>
where
    K: Clone + Ord,
    V: Interval<K>,
{
    pub(crate) const fn new() -> Self {
        Self {
            btree: BTreeMap::new(),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.btree.len()
    }

    pub(crate) fn insert(&mut self, item: V) {
        let range = item.range();
        self.btree.insert(range.start, item);
    }

    pub(crate) fn remove(&mut self, key: &K) -> Option<V> {
        self.btree.remove(key)
    }

    pub(crate) fn iter(&self) -> impl DoubleEndedIterator<Item = &V> {
        self.btree.values()
    }

    pub(crate) fn find_one(&self, point: &K) -> Option<&V> {
        let cursor = self.btree.lower_bound(core::ops::Bound::Excluded(point));
        cursor
            .peek_prev()
            .and_then(|(_, v)| (v.range().end > *point).then_some(v))
    }

    pub(crate) fn find<'a>(&'a self, range: &Range<K>) -> IntervalIter<'a, K, V> {
        let cursor = self
            .btree
            .lower_bound(core::ops::Bound::Excluded(&range.start));
        IntervalIter {
            cursor,
            range: range.clone(),
            peeked_prev: false,
        }
    }

    pub(crate) fn find_prev(&self, point: &K) -> Option<&V> {
        self.btree
            .upper_bound(core::ops::Bound::Excluded(point))
            .peek_prev()
            .map(|(_, v)| v)
    }

    pub(crate) fn find_next(&self, point: &K) -> Option<&V> {
        self.btree
            .lower_bound(core::ops::Bound::Excluded(point))
            .peek_next()
            .map(|(_, v)| v)
    }

    #[cfg(ktest)]
    pub(crate) fn take_one(&mut self, point: &K) -> Option<V> {
        let mut cursor = self
            .btree
            .lower_bound_mut(core::ops::Bound::Excluded(point));
        if let Some((_, v)) = cursor.peek_prev() {
            if v.range().end > *point {
                return Some(cursor.remove_prev().unwrap().1);
            }
        } else if let Some((_, v)) = cursor.peek_next()
            && v.range().start <= *point
        {
            return Some(cursor.remove_next().unwrap().1);
        }

        None
    }

    #[cfg(ktest)]
    pub(crate) fn take<'a>(&'a mut self, range: &Range<K>) -> IntervalDrain<'a, K, V> {
        let cursor = self
            .btree
            .lower_bound_mut(core::ops::Bound::Excluded(&range.start));
        IntervalDrain {
            cursor,
            range: range.clone(),
            drained_prev: false,
        }
    }

    pub(crate) fn clear(&mut self) {
        self.btree.clear();
    }
}

#[derive(Debug)]
pub(crate) struct IntervalIter<'a, K, V>
where
    K: Clone + Ord,
    V: Interval<K>,
{
    cursor: Cursor<'a, K, V>,
    range: Range<K>,
    peeked_prev: bool,
}

impl<'a, K, V> Iterator for IntervalIter<'a, K, V>
where
    K: Clone + Ord,
    V: Interval<K>,
{
    type Item = &'a V;

    fn next(&mut self) -> Option<Self::Item> {
        if !self.peeked_prev {
            self.peeked_prev = true;
            if let Some((_, v)) = self.cursor.peek_prev()
                && v.range().end > self.range.start
            {
                return Some(v);
            }
        }

        if let Some((_, v)) = self.cursor.next() {
            if v.range().start >= self.range.end {
                return None;
            }
            return Some(v);
        }

        None
    }
}

#[cfg_attr(not(ktest), expect(dead_code))]
#[derive(Debug)]
pub(crate) struct IntervalDrain<'a, K, V>
where
    K: Clone + Ord,
    V: Interval<K>,
{
    cursor: CursorMut<'a, K, V>,
    range: Range<K>,
    drained_prev: bool,
}

impl<K, V> Iterator for IntervalDrain<'_, K, V>
where
    K: Clone + Ord,
    V: Interval<K>,
{
    type Item = V;

    fn next(&mut self) -> Option<Self::Item> {
        if !self.drained_prev {
            self.drained_prev = true;
            if let Some((_, v)) = self.cursor.peek_prev()
                && v.range().end > self.range.start
            {
                return Some(self.cursor.remove_prev().unwrap().1);
            }
        }

        if let Some((_, v)) = self.cursor.peek_next() {
            if v.range().start >= self.range.end {
                return None;
            }
            return Some(self.cursor.remove_next().unwrap().1);
        }

        None
    }
}

#[cfg(ktest)]
mod tests {
    use alloc::{vec, vec::Vec};

    use ostd::prelude::ktest;

    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    struct TestInterval {
        range: Range<i32>,
    }

    impl Interval<i32> for TestInterval {
        fn range(&self) -> Range<i32> {
            self.range.clone()
        }
    }

    #[ktest]
    fn insert_and_find_one() {
        let mut set = IntervalSet::new();
        let interval = TestInterval { range: 10..20 };
        set.insert(interval.clone());

        assert_eq!(set.find_one(&15), Some(&interval));
        assert_eq!(set.find_one(&25), None);
    }

    #[ktest]
    fn remove() {
        let mut set = IntervalSet::new();
        let interval = TestInterval { range: 10..20 };
        set.insert(interval.clone());

        assert_eq!(set.remove(&10), Some(interval));
        assert_eq!(set.remove(&10), None);
    }

    #[ktest]
    fn iter() {
        let mut set = IntervalSet::new();
        let interval1 = TestInterval { range: 10..20 };
        let interval2 = TestInterval { range: 30..40 };
        set.insert(interval1.clone());
        set.insert(interval2.clone());

        let intervals: Vec<&TestInterval> = set.iter().collect();
        assert_eq!(intervals, vec![&interval1, &interval2]);
    }

    #[ktest]
    fn find() {
        let mut set = IntervalSet::new();
        let interval1 = TestInterval { range: 10..20 };
        let interval2 = TestInterval { range: 30..40 };
        let interval3 = TestInterval { range: 40..50 };
        let interval4 = TestInterval { range: 80..90 };
        set.insert(interval1.clone());
        set.insert(interval2.clone());
        set.insert(interval3.clone());
        set.insert(interval4.clone());

        let found: Vec<&TestInterval> = set.find(&(35..50)).collect();
        assert_eq!(found, vec![&interval2, &interval3]);
    }

    #[ktest]
    fn take_one() {
        let mut set = IntervalSet::new();
        let interval1 = TestInterval { range: 10..20 };
        let interval2 = TestInterval { range: 20..30 };
        set.insert(interval1.clone());
        set.insert(interval2.clone());

        assert_eq!(set.take_one(&15), Some(interval1));
        assert_eq!(set.take_one(&15), None);
    }

    #[ktest]
    fn take() {
        let mut set = IntervalSet::new();
        let interval1 = TestInterval { range: 10..20 };
        let interval2 = TestInterval { range: 30..40 };
        let interval3 = TestInterval { range: 45..50 };
        let interval4 = TestInterval { range: 60..70 };
        set.insert(interval1.clone());
        set.insert(interval2.clone());
        set.insert(interval3.clone());
        set.insert(interval4.clone());

        let taken: Vec<TestInterval> = set.take(&(35..45)).collect();
        assert_eq!(taken, vec![interval2]);
    }

    #[ktest]
    fn clear() {
        let mut set = IntervalSet::new();
        let interval1 = TestInterval { range: 10..20 };
        let interval2 = TestInterval { range: 20..30 };
        set.insert(interval1);
        set.insert(interval2);

        set.clear();
        assert!(set.iter().next().is_none());
    }
}
