use alloc::sync::Arc;
use core::ops::Range;

use bitvec::vec::BitVec;
use ostd::{
    Result,
    mm::{
        HasDaddr, HasSize, Infallible, PAGE_SIZE, VmReader, VmWriter,
        dma::{DmaDirection, DmaStream},
        io::util::{HasVmReaderWriter, VmReaderWriterResult},
    },
    sync::{LocalIrqDisabled, SpinLock},
};

#[derive(Debug)]
pub struct DmaArenaPool<D: DmaDirection> {
    storage: DmaStream<D>,
    manager: SpinLock<Manager, LocalIrqDisabled>,
}

#[derive(Debug)]
struct Manager {
    occupied: BitVec,
    min_free: usize,
}

impl<D: DmaDirection> DmaArenaPool<D> {
    pub fn new(capacity_pages: usize) -> Result<Arc<Self>> {
        if capacity_pages == 0 {
            return Err(ostd::Error::InvalidArgs);
        }

        Ok(Arc::new(Self {
            storage: DmaStream::alloc_uninit(capacity_pages, false)?,
            manager: SpinLock::new(Manager {
                occupied: BitVec::repeat(false, capacity_pages),
                min_free: 0,
            }),
        }))
    }

    pub fn alloc(self: &Arc<Self>, size_pages: usize) -> Option<DmaArena<D>> {
        let mut manager = self.manager.lock();
        let capacity_pages = manager.occupied.len();
        if size_pages == 0 || size_pages > capacity_pages - manager.min_free {
            return None;
        }

        let previous_min_free = manager.min_free;
        let (start, end) = {
            let mut start = previous_min_free;
            let mut end = start;
            while end - start < size_pages {
                if manager.occupied[end] {
                    start = end + 1;
                    if size_pages > capacity_pages - start {
                        return None;
                    }
                    end = start;
                } else {
                    end += 1;
                }
            }
            (start, end)
        };

        manager.occupied[start..end].fill(true);

        if manager.occupied[previous_min_free] {
            manager.min_free = manager.occupied[end..]
                .iter()
                .position(|occupied| !*occupied)
                .map(|position| end + position)
                .unwrap_or(capacity_pages);
        }

        Some(DmaArena {
            pool: self.clone(),
            page_range: start..end,
        })
    }

    fn free(&self, page_range: Range<usize>) {
        let mut manager = self.manager.lock();
        debug_assert!(manager.occupied[page_range.clone()].iter().all(|bit| *bit));
        manager.occupied[page_range.clone()].fill(false);
        manager.min_free = manager.min_free.min(page_range.start);
    }
}

#[derive(Debug)]
pub struct DmaArena<D: DmaDirection> {
    pool: Arc<DmaArenaPool<D>>,
    page_range: Range<usize>,
}

impl<D: DmaDirection> DmaArena<D> {
    fn byte_range(&self) -> Range<usize> {
        self.page_range.start * PAGE_SIZE..self.page_range.end * PAGE_SIZE
    }

    fn absolute_byte_range(&self, byte_range: Range<usize>) -> Result<Range<usize>> {
        if byte_range.start > byte_range.end || byte_range.end > self.size() {
            return Err(ostd::Error::InvalidArgs);
        }

        let arena_start = self.byte_range().start;
        Ok(arena_start + byte_range.start..arena_start + byte_range.end)
    }

    pub fn sync_from_device(&self, byte_range: Range<usize>) -> Result<()> {
        self.pool
            .storage
            .sync_from_device(self.absolute_byte_range(byte_range)?)
    }

    pub fn sync_to_device(&self, byte_range: Range<usize>) -> Result<()> {
        self.pool
            .storage
            .sync_to_device(self.absolute_byte_range(byte_range)?)
    }
}

impl<D: DmaDirection> Drop for DmaArena<D> {
    fn drop(&mut self) {
        self.pool.free(self.page_range.clone());
    }
}

impl<D: DmaDirection> HasSize for DmaArena<D> {
    fn size(&self) -> usize {
        self.page_range.len() * PAGE_SIZE
    }
}

impl<D: DmaDirection> HasDaddr for DmaArena<D> {
    fn daddr(&self) -> ostd::mm::Daddr {
        self.pool.storage.daddr() + self.byte_range().start
    }
}

impl<D: DmaDirection> HasVmReaderWriter for DmaArena<D> {
    type Types = VmReaderWriterResult;

    fn reader(&self) -> Result<VmReader<'_, Infallible>> {
        let byte_range = self.byte_range();
        let mut reader = self.pool.storage.reader()?;
        reader.skip(byte_range.start).limit(byte_range.len());
        Ok(reader)
    }

    fn writer(&self) -> Result<VmWriter<'_, Infallible>> {
        let byte_range = self.byte_range();
        let mut writer = self.pool.storage.writer()?;
        writer.skip(byte_range.start).limit(byte_range.len());
        Ok(writer)
    }
}

#[cfg(ktest)]
mod test {
    use ostd::{mm::dma::FromDevice, prelude::*};

    use super::*;

    const CAPACITY_PAGES: usize = 12;

    #[ktest]
    fn dropped_pages_are_reused() {
        let pool = DmaArenaPool::<FromDevice>::new(CAPACITY_PAGES).unwrap();
        let segment = pool.alloc(3).unwrap();
        let daddr = segment.daddr();
        assert_eq!(segment.size(), 3 * PAGE_SIZE);
        drop(segment);

        let reused_segment = pool.alloc(3).unwrap();
        assert_eq!(reused_segment.daddr(), daddr);
    }

    #[ktest]
    fn entire_arena_can_be_allocated() {
        let pool = DmaArenaPool::<FromDevice>::new(CAPACITY_PAGES).unwrap();
        let arena = pool.alloc(CAPACITY_PAGES).unwrap();
        assert_eq!(arena.size(), CAPACITY_PAGES * PAGE_SIZE);
        assert!(pool.alloc(1).is_none());
        drop(arena);

        assert!(pool.alloc(CAPACITY_PAGES).is_some());
    }

    #[ktest]
    fn skipped_free_range_remains_allocatable() {
        let pool = DmaArenaPool::<FromDevice>::new(CAPACITY_PAGES).unwrap();
        let first = pool.alloc(2).unwrap();
        let first_daddr = first.daddr();
        let _barrier = pool.alloc(1).unwrap();
        let _tail = pool.alloc(3).unwrap();
        drop(first);

        let _larger_than_gap = pool.alloc(3).unwrap();
        let reused_gap = pool.alloc(2).unwrap();
        assert_eq!(reused_gap.daddr(), first_daddr);
    }
}
