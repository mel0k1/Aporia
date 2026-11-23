#![no_std]

extern crate alloc;

use alloc::sync::Arc;
use core::{
    marker::PhantomData,
    num::Wrapping,
    ops::Deref,
    sync::atomic::{AtomicUsize, Ordering},
};

use inherit_methods_macro::inherit_methods;
use ostd::mm::{FrameAllocOptions, PAGE_SIZE, Segment, VmIo, io::util::HasVmReaderWriter};
use ostd_pod::Pod;

pub struct RingBuffer<T> {
    segment: Segment<()>,
    capacity: usize,
    tail: AtomicUsize,
    head: AtomicUsize,
    phantom: PhantomData<T>,
}

pub struct Producer<T, R: Deref<Target = RingBuffer<T>>> {
    rb: R,
    phantom: PhantomData<T>,
}

pub struct Consumer<T, R: Deref<Target = RingBuffer<T>>> {
    rb: R,
    phantom: PhantomData<T>,
}

pub type RbProducer<T> = Producer<T, Arc<RingBuffer<T>>>;

pub type RbConsumer<T> = Consumer<T, Arc<RingBuffer<T>>>;

impl<T> RingBuffer<T> {
    const T_SIZE: usize = size_of::<T>();

    pub fn new(capacity: usize) -> Self {
        assert!(
            capacity.is_power_of_two(),
            "capacity must be a power of two"
        );

        let nframes = capacity
            .checked_mul(Self::T_SIZE)
            .unwrap()
            .div_ceil(PAGE_SIZE);
        let segment = FrameAllocOptions::new()
            .zeroed(false)
            .alloc_segment(nframes)
            .unwrap();

        Self {
            segment,
            capacity,
            tail: AtomicUsize::new(0),
            head: AtomicUsize::new(0),
            phantom: PhantomData,
        }
    }

    pub fn split(self) -> (RbProducer<T>, RbConsumer<T>) {
        let producer = Producer {
            rb: Arc::new(self),
            phantom: PhantomData,
        };
        let consumer = Consumer {
            rb: Arc::clone(&producer.rb),
            phantom: PhantomData,
        };
        (producer, consumer)
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn segment(&self) -> &Segment<()> {
        &self.segment
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn is_full(&self) -> bool {
        self.free_len() == 0
    }

    pub fn len(&self) -> usize {
        (self.tail() - self.head()).0
    }

    pub fn free_len(&self) -> usize {
        self.capacity - self.len()
    }

    pub fn head(&self) -> Wrapping<usize> {
        Wrapping(self.head.load(Ordering::Acquire))
    }

    pub fn tail(&self) -> Wrapping<usize> {
        Wrapping(self.tail.load(Ordering::Acquire))
    }

    pub(crate) fn advance_tail(&self, mut tail: Wrapping<usize>, len: usize) {
        tail += len;
        self.tail.store(tail.0, Ordering::Release);
    }

    pub(crate) fn advance_head(&self, mut head: Wrapping<usize>, len: usize) {
        head += len;
        self.head.store(head.0, Ordering::Release);
    }

    pub(crate) fn reset_head(&self) {
        let new_head = self.tail();
        self.head.store(new_head.0, Ordering::Release);
    }

    pub fn clear(&mut self) {
        self.tail.store(0, Ordering::Release);
        self.head.store(0, Ordering::Release);
    }
}

impl RingBuffer<u8> {
    pub fn commit_read(&mut self, len: usize) {
        assert!(
            len <= self.len(),
            "commit_read: len exceeds available items"
        );
        let head = self.head();
        self.advance_head(head, len);
    }
}

impl<T: Pod> RingBuffer<T> {
    pub fn push(&mut self, item: T) -> Option<()> {
        let mut producer = Producer {
            rb: self,
            phantom: PhantomData,
        };
        producer.push(item)
    }

    pub fn push_slice(&mut self, items: &[T]) -> Option<()> {
        let mut producer = Producer {
            rb: self,
            phantom: PhantomData,
        };
        producer.push_slice(items)
    }

    pub fn pop(&mut self) -> Option<T> {
        let mut consumer = Consumer {
            rb: self,
            phantom: PhantomData,
        };
        consumer.pop()
    }

    pub fn pop_slice(&mut self, items: &mut [T]) -> Option<()> {
        let mut consumer = Consumer {
            rb: self,
            phantom: PhantomData,
        };
        consumer.pop_slice(items)
    }
}

impl<T: Pod, R: Deref<Target = RingBuffer<T>>> Producer<T, R> {
    const T_SIZE: usize = size_of::<T>();

    pub fn push(&mut self, item: T) -> Option<()> {
        let rb = &self.rb;
        if rb.is_full() {
            return None;
        }

        let tail = rb.tail();
        let offset = tail.0 & (rb.capacity - 1);
        let byte_offset = offset * Self::T_SIZE;

        let mut writer = rb.segment.writer();
        writer.skip(byte_offset);
        writer.write_val(&item).unwrap();

        rb.advance_tail(tail, 1);
        Some(())
    }

    pub fn push_slice(&mut self, items: &[T]) -> Option<()> {
        let rb = &self.rb;
        let nitems = items.len();
        if rb.free_len() < nitems {
            return None;
        }

        let tail = rb.tail();
        let offset = tail.0 & (rb.capacity - 1);
        let byte_offset = offset * Self::T_SIZE;

        if offset + nitems > rb.capacity {
            rb.segment
                .write_slice(byte_offset, &items[..rb.capacity - offset])
                .unwrap();
            rb.segment
                .write_slice(0, &items[rb.capacity - offset..])
                .unwrap();
        } else {
            rb.segment.write_slice(byte_offset, items).unwrap();
        }

        rb.advance_tail(tail, nitems);
        Some(())
    }
}

#[inherit_methods(from = "self.rb")]
impl<T, R: Deref<Target = RingBuffer<T>>> Producer<T, R> {
    pub fn capacity(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn is_full(&self) -> bool;
    pub fn len(&self) -> usize;
    pub fn free_len(&self) -> usize;
    pub fn head(&self) -> Wrapping<usize>;
    pub fn tail(&self) -> Wrapping<usize>;
    pub fn segment(&self) -> &Segment<()>;
}

impl<T, R: Deref<Target = RingBuffer<T>>> Producer<T, R> {
    pub fn commit_write(&self, len: usize) {
        assert!(
            len <= self.free_len(),
            "commit_write: len exceeds free space"
        );
        let tail = self.tail();
        self.rb.advance_tail(tail, len);
    }
}

impl<T: Pod, R: Deref<Target = RingBuffer<T>>> Consumer<T, R> {
    const T_SIZE: usize = size_of::<T>();

    pub fn pop(&mut self) -> Option<T> {
        let rb = &self.rb;
        if rb.is_empty() {
            return None;
        }

        let head = rb.head();
        let offset = head.0 & (rb.capacity - 1);
        let byte_offset = offset * Self::T_SIZE;

        let mut reader = rb.segment.reader();
        reader.skip(byte_offset);
        let item = reader.read_val::<T>().unwrap();

        rb.advance_head(head, 1);
        Some(item)
    }

    pub fn pop_slice(&mut self, items: &mut [T]) -> Option<()> {
        let rb = &self.rb;
        let nitems = items.len();
        if rb.len() < nitems {
            return None;
        }

        let head = rb.head();
        let offset = head.0 & (rb.capacity - 1);
        let byte_offset = offset * Self::T_SIZE;

        if offset + nitems > rb.capacity {
            rb.segment
                .read_slice(byte_offset, &mut items[..rb.capacity - offset])
                .unwrap();
            rb.segment
                .read_slice(0, &mut items[rb.capacity - offset..])
                .unwrap();
        } else {
            rb.segment.read_slice(byte_offset, items).unwrap();
        }

        rb.advance_head(head, nitems);
        Some(())
    }

    pub fn skip(&mut self, count: usize) {
        let rb = &self.rb;
        let len = rb.len();
        assert!(len >= count, "skip: count exceeds available items");

        let head = rb.head();
        rb.advance_head(head, count);
    }

    pub fn clear(&mut self) {
        self.rb.reset_head();
    }
}

#[inherit_methods(from = "self.rb")]
impl<T, R: Deref<Target = RingBuffer<T>>> Consumer<T, R> {
    pub fn capacity(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn is_full(&self) -> bool;
    pub fn len(&self) -> usize;
    pub fn free_len(&self) -> usize;
    pub fn head(&self) -> Wrapping<usize>;
    pub fn tail(&self) -> Wrapping<usize>;
    pub fn segment(&self) -> &Segment<()>;
}

impl<T, R: Deref<Target = RingBuffer<T>>> Consumer<T, R> {
    pub fn ring_buffer(&self) -> &RingBuffer<T> {
        &self.rb
    }

    pub fn commit_read(&mut self, len: usize) {
        assert!(
            len <= self.len(),
            "commit_read: len exceeds available items"
        );
        let head = self.head();
        self.rb.advance_head(head, len);
    }
}

#[cfg(ktest)]
mod test;
