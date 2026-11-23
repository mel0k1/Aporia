use core::{
    fmt::Debug,
    marker::PhantomData,
    ops::{Bound, Range, RangeFrom, RangeFull, RangeTo, RangeToInclusive},
    sync::atomic::{AtomicU64, Ordering},
};

use bitvec::{order::Lsb0, view::BitView};
use smallvec::SmallVec;

use crate::const_assert;

pub unsafe trait Id: Copy + Clone + Debug + Eq + Into<u32> + PartialEq {
    fn new(raw_id: u32) -> Self {
        assert!(raw_id < Self::cardinality());
        unsafe { Self::new_unchecked(raw_id) }
    }

    unsafe fn new_unchecked(raw_id: u32) -> Self;

    fn cardinality() -> u32;

    fn as_usize(self) -> usize {
        Into::<u32>::into(self) as usize
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdSet<I> {
    bits: SmallVec<[InnerPart; NR_PARTS_NO_ALLOC]>,
    phantom: PhantomData<I>,
}

type InnerPart = u64;

const BITS_PER_PART: usize = InnerPart::BITS as usize;
const NR_PARTS_NO_ALLOC: usize = 2;

fn part_idx<I: Id>(id: I) -> usize {
    (id.into() as usize) / BITS_PER_PART
}

fn bit_idx<I: Id>(id: I) -> usize {
    (id.into() as usize) % BITS_PER_PART
}

fn parts_for_ids<I: Id>() -> usize {
    (I::cardinality() as usize).div_ceil(BITS_PER_PART)
}

impl<I: Id> IdSet<I> {
    pub fn new_full() -> Self {
        let mut bits = Self::with_bit_pattern(!0);
        Self::clear_invalid_id_bits(&mut bits);
        Self {
            bits,
            phantom: PhantomData,
        }
    }

    pub fn new_empty() -> Self {
        let bits = Self::with_bit_pattern(0);
        Self {
            bits,
            phantom: PhantomData,
        }
    }

    fn with_bit_pattern(part_bits: InnerPart) -> SmallVec<[InnerPart; NR_PARTS_NO_ALLOC]> {
        let num_parts = parts_for_ids::<I>();
        let mut bits = SmallVec::with_capacity(num_parts);
        bits.resize(num_parts, part_bits);
        bits
    }

    fn clear_invalid_id_bits(bits: &mut SmallVec<[InnerPart; NR_PARTS_NO_ALLOC]>) {
        let num_ids = I::cardinality() as usize;
        if !num_ids.is_multiple_of(BITS_PER_PART) {
            let num_parts = parts_for_ids::<I>();
            bits[num_parts - 1] &= (1 << (num_ids % BITS_PER_PART)) - 1;
        }
    }

    pub fn add(&mut self, id: I) {
        let part_idx = part_idx(id);
        let bit_idx = bit_idx(id);
        if part_idx >= self.bits.len() {
            self.bits.resize(part_idx + 1, 0);
        }
        self.bits[part_idx] |= 1 << bit_idx;
    }

    pub fn remove(&mut self, id: I) {
        let part_idx = part_idx(id);
        let bit_idx = bit_idx(id);
        if part_idx < self.bits.len() {
            self.bits[part_idx] &= !(1 << bit_idx);
        }
    }

    pub fn contains(&self, id: I) -> bool {
        let part_idx = part_idx(id);
        let bit_idx = bit_idx(id);
        part_idx < self.bits.len() && (self.bits[part_idx] & (1 << bit_idx)) != 0
    }

    pub fn count(&self) -> usize {
        self.bits
            .iter()
            .map(|part| part.count_ones() as usize)
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        self.bits.iter().all(|part| *part == 0)
    }

    pub fn is_full(&self) -> bool {
        let num_ids = I::cardinality() as usize;
        self.bits.iter().enumerate().all(|(idx, part)| {
            if idx == self.bits.len() - 1 && !num_ids.is_multiple_of(BITS_PER_PART) {
                *part == (1 << (num_ids % BITS_PER_PART)) - 1
            } else {
                *part == !0
            }
        })
    }

    pub fn add_all(&mut self) {
        self.bits.fill(!0);
        Self::clear_invalid_id_bits(&mut self.bits);
    }

    pub fn clear(&mut self) {
        self.bits.fill(0);
    }

    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = I> + '_ {
        self.iter_in(..)
    }

    pub fn iter_in<S: IdSetSlicer<I>>(&self, slicer: S) -> impl Iterator<Item = I> + '_ {
        let (start, end) = slicer.to_range_bounds();

        self.bits.view_bits::<Lsb0>()[start..end]
            .iter_ones()
            .map(move |offset| unsafe { I::new_unchecked((start + offset) as u32) })
    }
}

pub trait IdSetSlicer<I: Id> {
    fn to_range_bounds(self) -> (usize, usize);
}

impl<I: Id> IdSetSlicer<I> for RangeTo<I> {
    fn to_range_bounds(self) -> (usize, usize) {
        (0, self.end.as_usize())
    }
}
impl<I: Id> IdSetSlicer<I> for RangeFrom<I> {
    fn to_range_bounds(self) -> (usize, usize) {
        (self.start.as_usize(), I::cardinality() as usize)
    }
}
impl<I: Id> IdSetSlicer<I> for Range<I> {
    fn to_range_bounds(self) -> (usize, usize) {
        (self.start.as_usize(), self.end.as_usize())
    }
}
impl<I: Id> IdSetSlicer<I> for RangeFull {
    fn to_range_bounds(self) -> (usize, usize) {
        (0, I::cardinality() as usize)
    }
}
impl<I: Id> IdSetSlicer<I> for RangeToInclusive<I> {
    fn to_range_bounds(self) -> (usize, usize) {
        (0, self.end.as_usize() + 1)
    }
}
impl<I: Id> IdSetSlicer<I> for (Bound<I>, Bound<I>) {
    fn to_range_bounds(self) -> (usize, usize) {
        let (start_bound, end_bound) = self;
        let start = match start_bound {
            Bound::Included(id) => id.as_usize(),
            Bound::Excluded(id) => id.as_usize() + 1,
            Bound::Unbounded => 0,
        };
        let end = match end_bound {
            Bound::Included(id) => id.as_usize() + 1,
            Bound::Excluded(id) => id.as_usize(),
            Bound::Unbounded => I::cardinality() as usize,
        };
        (start, end)
    }
}

impl<I: Id> From<I> for IdSet<I> {
    fn from(id: I) -> Self {
        let mut set = Self::new_empty();
        set.add(id);
        set
    }
}

impl<I: Id> Default for IdSet<I> {
    fn default() -> Self {
        Self::new_empty()
    }
}

#[derive(Debug)]
pub struct AtomicIdSet<I> {
    bits: SmallVec<[AtomicInnerPart; NR_PARTS_NO_ALLOC]>,
    phantom: PhantomData<I>,
}

type AtomicInnerPart = AtomicU64;
const_assert!(size_of::<AtomicInnerPart>() == size_of::<InnerPart>());

impl<I: Id> AtomicIdSet<I> {
    pub fn new(value: IdSet<I>) -> Self {
        let bits = value.bits.into_iter().map(AtomicU64::new).collect();
        Self {
            bits,
            phantom: PhantomData,
        }
    }

    pub fn load(&self, ordering: Ordering) -> IdSet<I> {
        let bits = self
            .bits
            .iter()
            .map(|part| match ordering {
                Ordering::Release => part.fetch_or(0, ordering),
                _ => part.load(ordering),
            })
            .collect();
        IdSet {
            bits,
            phantom: PhantomData,
        }
    }

    pub fn store(&self, value: &IdSet<I>, ordering: Ordering) {
        for (part, new_part) in self.bits.iter().zip(value.bits.iter()) {
            part.store(*new_part, ordering);
        }
    }

    pub fn add(&self, id: I, ordering: Ordering) {
        let part_idx = part_idx(id);
        let bit_idx = bit_idx(id);
        if part_idx < self.bits.len() {
            self.bits[part_idx].fetch_or(1 << bit_idx, ordering);
        }
    }

    pub fn remove(&self, id: I, ordering: Ordering) {
        let part_idx = part_idx(id);
        let bit_idx = bit_idx(id);
        if part_idx < self.bits.len() {
            self.bits[part_idx].fetch_and(!(1 << bit_idx), ordering);
        }
    }

    pub fn contains(&self, id: I, ordering: Ordering) -> bool {
        let part_idx = part_idx(id);
        let bit_idx = bit_idx(id);
        part_idx < self.bits.len() && (self.bits[part_idx].load(ordering) & (1 << bit_idx)) != 0
    }
}

#[cfg(ktest)]
mod id_set_tests {
    use alloc::vec;

    use super::*;
    use crate::prelude::*;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct MockId<const C: u32>(u32);

    unsafe impl<const C: u32> Id for MockId<C> {
        unsafe fn new_unchecked(raw_id: u32) -> Self {
            MockId(raw_id)
        }

        fn cardinality() -> u32 {
            C
        }
    }

    impl<const C: u32> From<MockId<C>> for u32 {
        fn from(id: MockId<C>) -> u32 {
            id.0
        }
    }

    #[ktest]
    fn id_set_empty() {
        type TestId = MockId<10>;
        let set: IdSet<TestId> = IdSet::new_empty();
        assert!(set.is_empty());
        assert_eq!(set.count(), 0);
        for i in 0..10 {
            assert!(!set.contains(TestId::new(i)));
        }
    }

    #[ktest]
    fn id_set_full() {
        type TestId = MockId<10>;
        let set: IdSet<TestId> = IdSet::new_full();
        assert!(!set.is_empty());
        assert_eq!(set.count(), 10);
        for i in 0..10 {
            assert!(set.contains(TestId::new(i)));
        }
    }

    #[ktest]
    fn id_set_add_remove() {
        type TestId = MockId<64>;
        let mut set: IdSet<TestId> = IdSet::new_empty();

        assert!(set.is_empty());

        set.add(TestId::new(0));
        assert!(set.contains(TestId::new(0)));
        assert_eq!(set.count(), 1);
        assert!(!set.is_empty());

        set.add(TestId::new(63));
        assert!(set.contains(TestId::new(63)));
        assert_eq!(set.count(), 2);

        set.add(TestId::new(32));
        assert!(set.contains(TestId::new(32)));
        assert_eq!(set.count(), 3);

        set.remove(TestId::new(0));
        assert!(!set.contains(TestId::new(0)));
        assert_eq!(set.count(), 2);

        set.remove(TestId::new(63));
        assert!(!set.contains(TestId::new(63)));
        assert_eq!(set.count(), 1);

        set.remove(TestId::new(32));
        assert!(!set.contains(TestId::new(32)));
        assert_eq!(set.count(), 0);
        assert!(set.is_empty());

        set.remove(TestId::new(1));
        assert!(set.is_empty());
    }

    #[ktest]
    fn id_set_add_remove_multi_part() {
        type TestId = MockId<128>;
        let mut set: IdSet<TestId> = IdSet::new_empty();

        set.add(TestId::new(0));
        set.add(TestId::new(63));
        set.add(TestId::new(64));
        set.add(TestId::new(127));

        assert_eq!(set.count(), 4);
        assert!(set.contains(TestId::new(0)));
        assert!(set.contains(TestId::new(63)));
        assert!(set.contains(TestId::new(64)));
        assert!(set.contains(TestId::new(127)));

        set.remove(TestId::new(63));
        assert!(!set.contains(TestId::new(63)));
        assert_eq!(set.count(), 3);

        set.remove(TestId::new(64));
        assert!(!set.contains(TestId::new(64)));
        assert_eq!(set.count(), 2);
    }

    #[ktest]
    fn id_set_add_all_clear() {
        type TestId = MockId<70>;
        let mut set: IdSet<TestId> = IdSet::new_empty();

        set.add_all();
        assert_eq!(set.count(), 70);
        assert!(set.is_full());
        for i in 0..70 {
            assert!(set.contains(TestId::new(i)));
        }

        set.clear();
        assert!(set.is_empty());
        assert_eq!(set.count(), 0);
        for i in 0..70 {
            assert!(!set.contains(TestId::new(i)));
        }
    }

    #[ktest]
    fn id_set_iter() {
        type TestId = MockId<5>;
        let mut set: IdSet<TestId> = IdSet::new_empty();

        set.add(TestId::new(2));
        set.add(TestId::new(0));
        set.add(TestId::new(4));

        let collected_ids: Vec<TestId> = set.iter().collect();
        assert_eq!(
            collected_ids,
            vec![TestId::new(0), TestId::new(2), TestId::new(4)]
        );

        set.clear();
        let collected_ids: Vec<TestId> = set.iter().collect();
        assert!(collected_ids.is_empty());
    }

    #[ktest]
    fn id_set_iter_full() {
        type TestId = MockId<3>;
        let set: IdSet<TestId> = IdSet::new_full();
        let collected_ids: Vec<TestId> = set.iter().collect();
        assert_eq!(
            collected_ids,
            vec![TestId::new(0), TestId::new(1), TestId::new(2)]
        );
    }

    #[ktest]
    fn id_set_iter_multi_part() {
        type TestId = MockId<100>;
        let mut set: IdSet<TestId> = IdSet::new_empty();
        set.add(TestId::new(1));
        set.add(TestId::new(65));
        set.add(TestId::new(99));
        set.add(TestId::new(0));
        set.add(TestId::new(63));

        let collected_ids: Vec<TestId> = set.iter().collect();
        assert_eq!(
            collected_ids,
            vec![
                TestId::new(0),
                TestId::new(1),
                TestId::new(63),
                TestId::new(65),
                TestId::new(99)
            ]
        );
    }

    #[ktest]
    fn id_set_from_id() {
        type TestId = MockId<10>;
        let id = TestId::new(5);
        let set: IdSet<TestId> = id.into();
        assert_eq!(set.count(), 1);
        assert!(set.contains(id));
        assert!(!set.contains(TestId::new(0)));
    }

    #[ktest]
    fn id_set_cardinality_one() {
        type TestId = MockId<1>;
        let mut set: IdSet<TestId> = IdSet::new_empty();
        assert!(set.is_empty());
        assert_eq!(set.count(), 0);

        set.add(TestId::new(0));
        assert!(set.contains(TestId::new(0)));
        assert_eq!(set.count(), 1);
        assert!(set.is_full());

        set.remove(TestId::new(0));
        assert!(!set.contains(TestId::new(0)));
        assert_eq!(set.count(), 0);
        assert!(set.is_empty());

        let full_set = IdSet::<TestId>::new_full();
        assert!(full_set.contains(TestId::new(0)));
        assert_eq!(full_set.count(), 1);
    }

    #[ktest]
    fn id_set_exact_part_boundary() {
        type TestId = MockId<64>;
        let mut set: IdSet<TestId> = IdSet::new_empty();

        set.add(TestId::new(0));
        set.add(TestId::new(63));
        assert_eq!(set.count(), 2);

        let full_set = IdSet::<TestId>::new_full();
        assert!(full_set.is_full());
        assert_eq!(full_set.count(), 64);
        for i in 0..64 {
            assert!(full_set.contains(TestId::new(i)));
        }
    }

    #[ktest]
    fn id_set_just_over_part_boundary() {
        type TestId = MockId<65>;
        let mut set: IdSet<TestId> = IdSet::new_empty();

        set.add(TestId::new(0));
        set.add(TestId::new(63));
        set.add(TestId::new(64));
        assert_eq!(set.count(), 3);

        let full_set = IdSet::<TestId>::new_full();
        assert!(full_set.is_full());
        assert_eq!(full_set.count(), 65);
        for i in 0..65 {
            assert!(full_set.contains(TestId::new(i)));
        }
    }

    #[ktest]
    fn id_set_is_full_with_less_than_full_last_part() {
        type TestId = MockId<70>;
        let mut set: IdSet<TestId> = IdSet::new_full();

        assert!(set.is_full());
        assert_eq!(set.count(), 70);

        set.remove(TestId::new(69));
        assert!(!set.is_full());
        assert_eq!(set.count(), 69);

        set.add(TestId::new(69));
        assert!(set.is_full());
        assert_eq!(set.count(), 70);
    }

    #[ktest]
    fn id_set_default() {
        type TestId = MockId<10>;
        let set: IdSet<TestId> = Default::default();
        assert!(set.is_empty());
        assert_eq!(set.count(), 0);
    }

    #[ktest]
    fn iter_in_range() {
        type TestId = MockId<7>;
        let mut set: IdSet<TestId> = IdSet::new_empty();
        set.add(TestId::new(0));
        set.add(TestId::new(1));
        set.add(TestId::new(2));
        set.add(TestId::new(5));
        set.add(TestId::new(6));

        let collected_ids: Vec<TestId> = set.iter_in(TestId::new(1)..TestId::new(5)).collect();
        assert_eq!(collected_ids, vec![TestId::new(1), TestId::new(2)],);
    }

    #[ktest]
    fn iter_in_range_to() {
        type TestId = MockId<7>;
        let mut set: IdSet<TestId> = IdSet::new_empty();
        set.add(TestId::new(0));
        set.add(TestId::new(1));
        set.add(TestId::new(2));
        set.add(TestId::new(5));
        set.add(TestId::new(6));

        let collected_ids: Vec<TestId> = set.iter_in(..TestId::new(5)).collect();
        assert_eq!(
            collected_ids,
            vec![TestId::new(0), TestId::new(1), TestId::new(2)],
        );
    }

    #[ktest]
    fn iter_in_range_to_inclusive() {
        type TestId = MockId<7>;
        let mut set: IdSet<TestId> = IdSet::new_empty();
        set.add(TestId::new(0));
        set.add(TestId::new(1));
        set.add(TestId::new(2));
        set.add(TestId::new(5));
        set.add(TestId::new(6));

        let collected_ids: Vec<TestId> = set.iter_in(..=TestId::new(5)).collect();
        assert_eq!(
            collected_ids,
            vec![
                TestId::new(0),
                TestId::new(1),
                TestId::new(2),
                TestId::new(5)
            ],
        );
    }

    #[ktest]
    fn iter_in_range_from() {
        type TestId = MockId<7>;
        let mut set: IdSet<TestId> = IdSet::new_empty();
        set.add(TestId::new(0));
        set.add(TestId::new(1));
        set.add(TestId::new(2));
        set.add(TestId::new(5));
        set.add(TestId::new(6));

        let collected_ids: Vec<TestId> = set.iter_in(TestId::new(2)..).collect();
        assert_eq!(
            collected_ids,
            vec![TestId::new(2), TestId::new(5), TestId::new(6)],
        );
    }

    #[ktest]
    fn iter_in_range_full() {
        type TestId = MockId<7>;
        let mut set: IdSet<TestId> = IdSet::new_empty();
        set.add(TestId::new(0));
        set.add(TestId::new(1));
        set.add(TestId::new(2));
        set.add(TestId::new(5));
        set.add(TestId::new(6));

        let collected_ids: Vec<TestId> = set.iter_in(..).collect();
        assert_eq!(
            collected_ids,
            vec![
                TestId::new(0),
                TestId::new(1),
                TestId::new(2),
                TestId::new(5),
                TestId::new(6)
            ],
        );
    }

    #[ktest]
    fn iter_in_bound_tuple_inclusive_exclusive() {
        type TestId = MockId<7>;
        let mut set: IdSet<TestId> = IdSet::new_empty();
        set.add(TestId::new(0));
        set.add(TestId::new(1));
        set.add(TestId::new(2));
        set.add(TestId::new(5));
        set.add(TestId::new(6));

        let collected_ids: Vec<TestId> = set
            .iter_in((
                Bound::Included(TestId::new(1)),
                Bound::Excluded(TestId::new(5)),
            ))
            .collect();
        assert_eq!(collected_ids, vec![TestId::new(1), TestId::new(2)],);
    }

    #[ktest]
    fn iter_in_bound_tuple_exclusive_inclusive() {
        type TestId = MockId<7>;
        let mut set: IdSet<TestId> = IdSet::new_empty();
        set.add(TestId::new(0));
        set.add(TestId::new(1));
        set.add(TestId::new(2));
        set.add(TestId::new(5));
        set.add(TestId::new(6));

        let collected_ids: Vec<TestId> = set
            .iter_in((
                Bound::Excluded(TestId::new(1)),
                Bound::Included(TestId::new(5)),
            ))
            .collect();
        assert_eq!(collected_ids, vec![TestId::new(2), TestId::new(5)],);
    }

    #[ktest]
    fn iter_in_unbounded_bounds() {
        type TestId = MockId<7>;
        let mut set: IdSet<TestId> = IdSet::new_empty();
        set.add(TestId::new(0));
        set.add(TestId::new(1));
        set.add(TestId::new(2));
        set.add(TestId::new(5));
        set.add(TestId::new(6));

        let collected_ids: Vec<TestId> = set
            .iter_in((Bound::Unbounded::<TestId>, Bound::Unbounded::<TestId>))
            .collect();
        assert_eq!(
            collected_ids,
            vec![
                TestId::new(0),
                TestId::new(1),
                TestId::new(2),
                TestId::new(5),
                TestId::new(6)
            ],
        );
    }

    #[ktest]
    fn iter_in_half_unbounded() {
        type TestId = MockId<7>;
        let mut set: IdSet<TestId> = IdSet::new_empty();
        set.add(TestId::new(0));
        set.add(TestId::new(1));
        set.add(TestId::new(2));
        set.add(TestId::new(5));
        set.add(TestId::new(6));

        let collected_ids: Vec<TestId> = set
            .iter_in((Bound::Included(TestId::new(2)), Bound::Unbounded::<TestId>))
            .collect();
        assert_eq!(
            collected_ids,
            vec![TestId::new(2), TestId::new(5), TestId::new(6)],
        );

        let collected_ids: Vec<TestId> = set
            .iter_in((Bound::Unbounded::<TestId>, Bound::Included(TestId::new(2))))
            .collect();
        assert_eq!(
            collected_ids,
            vec![TestId::new(0), TestId::new(1), TestId::new(2)],
        );
    }

    #[ktest]
    fn iter_in_range_starts_after_last() {
        type TestId = MockId<7>;
        let mut set: IdSet<TestId> = IdSet::new_empty();
        set.add(TestId::new(0));
        set.add(TestId::new(1));
        set.add(TestId::new(2));

        let collected_ids: Vec<TestId> = set.iter_in(TestId::new(3)..).collect();
        assert_eq!(collected_ids, vec![],);
    }

    #[ktest]
    fn iter_in_range_ends_after_last() {
        type TestId = MockId<7>;
        let mut set: IdSet<TestId> = IdSet::new_empty();
        set.add(TestId::new(0));
        set.add(TestId::new(1));
        set.add(TestId::new(2));

        let collected_ids: Vec<TestId> = set.iter_in(..TestId::new(3)).collect();
        assert_eq!(
            collected_ids,
            vec![TestId::new(0), TestId::new(1), TestId::new(2)],
        );
    }

    #[ktest]
    fn iter_in_range_next_part() {
        type TestId = MockId<{ InnerPart::BITS }>;
        let last_id = TestId::new(InnerPart::BITS - 1);

        let mut set: IdSet<TestId> = IdSet::new_empty();
        set.add(last_id);

        let collected_ids: Vec<TestId> = set
            .iter_in((Bound::Excluded(last_id), Bound::Included(last_id)))
            .collect();
        assert_eq!(collected_ids, vec![],);
    }
}
