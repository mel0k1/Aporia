#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::{
    collections::VecDeque,
    sync::{Arc, Weak},
};
use core::ops::Range;

use aster_softirq::BottomHalfDisabled;
use aster_util::mem_obj_slice::Slice;
use bitvec::array::BitArray;
use ostd::{
    mm::{
        Daddr, FrameAllocOptions, HasDaddr, Infallible, PAGE_SIZE, USegment, VmReader, VmWriter,
        dma::{DmaDirection, DmaStream},
        io::util::HasVmReaderWriter,
    },
    sync::SpinLock,
};

mod arena;
mod buffer;

pub use self::{
    arena::{DmaArena, DmaArenaPool},
    buffer::DmaBuffer,
};

#[derive(Debug)]
pub struct DmaPool<D: DmaDirection> {
    segment_size: usize,
    is_cache_coherent: bool,
    high_watermark: usize,
    avail_pages: SpinLock<VecDeque<Arc<DmaPage<D>>>, BottomHalfDisabled>,
    all_pages: SpinLock<VecDeque<Arc<DmaPage<D>>>, BottomHalfDisabled>,
}

impl<D: DmaDirection> DmaPool<D> {
    pub fn new(
        segment_size: usize,
        init_size: usize,
        high_watermark: usize,
        is_cache_coherent: bool,
    ) -> Arc<Self> {
        assert!(segment_size.is_power_of_two());
        assert!(segment_size >= 64);
        assert!(segment_size <= PAGE_SIZE);
        assert!(high_watermark >= init_size);

        Arc::new_cyclic(|pool| {
            let mut avail_pages = VecDeque::new();
            let mut all_pages = VecDeque::new();

            for _ in 0..init_size {
                let page = Arc::new(
                    DmaPage::new(segment_size, is_cache_coherent, Weak::clone(pool)).unwrap(),
                );
                avail_pages.push_back(page.clone());
                all_pages.push_back(page);
            }

            Self {
                segment_size,
                is_cache_coherent,
                high_watermark,
                avail_pages: SpinLock::new(avail_pages),
                all_pages: SpinLock::new(all_pages),
            }
        })
    }

    pub fn alloc_segment(self: &Arc<Self>) -> Result<DmaSegment<D>, ostd::Error> {
        let mut avail_pages = self.avail_pages.lock();
        if avail_pages.is_empty() {
            let new_page = {
                let pool = Arc::downgrade(self);
                Arc::new(DmaPage::new(
                    self.segment_size,
                    self.is_cache_coherent,
                    pool,
                )?)
            };
            let mut all_pages = self.all_pages.lock();
            avail_pages.push_back(new_page.clone());
            all_pages.push_back(new_page);
        }

        let first_avail_page = avail_pages.front().unwrap();
        let free_segment = first_avail_page.alloc_segment().unwrap();
        if first_avail_page.is_full() {
            avail_pages.pop_front();
        }
        Ok(free_segment)
    }

    #[cfg(ktest)]
    fn num_pages(&self) -> usize {
        self.all_pages.lock().len()
    }

    pub fn segment_size(&self) -> usize {
        self.segment_size
    }
}

#[derive(Debug)]
struct DmaPage<D: DmaDirection> {
    storage: USegment,
    dma_stream: DmaStream<D>,
    segment_size: usize,
    allocated_segments: SpinLock<BitArray, BottomHalfDisabled>,
    pool: Weak<DmaPool<D>>,
}

impl<D: DmaDirection> DmaPage<D> {
    fn new(
        segment_size: usize,
        is_cache_coherent: bool,
        pool: Weak<DmaPool<D>>,
    ) -> Result<Self, ostd::Error> {
        let storage: USegment = FrameAllocOptions::new().alloc_segment(1)?.into();
        let dma_stream = DmaStream::map(storage.clone(), is_cache_coherent)?;

        Ok(Self {
            storage,
            dma_stream,
            segment_size,
            allocated_segments: SpinLock::new(BitArray::ZERO),
            pool,
        })
    }

    fn alloc_segment(self: &Arc<Self>) -> Option<DmaSegment<D>> {
        let mut segments = self.allocated_segments.lock();
        let free_segment_index = get_next_free_index(&segments, self.nr_blocks_per_page())?;
        segments.set(free_segment_index, true);

        let segment = DmaSegment {
            page: self.clone(),
            offset: free_segment_index * self.segment_size,
        };
        Some(segment)
    }

    const fn nr_blocks_per_page(&self) -> usize {
        PAGE_SIZE / self.segment_size
    }

    fn is_full(&self) -> bool {
        let segments = self.allocated_segments.lock();
        get_next_free_index(&segments, self.nr_blocks_per_page()).is_none()
    }
}

fn get_next_free_index(segments: &BitArray, nr_blocks_per_page: usize) -> Option<usize> {
    let free_segment_index = segments.iter_zeros().next()?;

    if free_segment_index >= nr_blocks_per_page {
        None
    } else {
        Some(free_segment_index)
    }
}

#[derive(Debug)]
pub struct DmaSegment<D: DmaDirection> {
    page: Arc<DmaPage<D>>,
    offset: usize,
}

impl<D: DmaDirection> HasDaddr for DmaSegment<D> {
    fn daddr(&self) -> Daddr {
        self.page.dma_stream.daddr() + self.offset
    }
}

impl<D: DmaDirection> DmaSegment<D> {
    pub fn size(&self) -> usize {
        self.page.segment_size
    }

    pub fn storage(&self) -> Slice<USegment> {
        Slice::new(
            self.page.storage.clone(),
            self.offset..self.offset + self.page.segment_size,
        )
    }

    pub fn reader(&self) -> Result<VmReader<'_, Infallible>, ostd::Error> {
        if !D::CAN_READ_FROM_DEVICE {
            return Err(ostd::Error::AccessDenied);
        }
        let mut reader = self.page.storage.reader();
        reader.skip(self.offset).limit(self.page.segment_size);
        Ok(reader)
    }

    pub fn writer(&self) -> Result<VmWriter<'_, Infallible>, ostd::Error> {
        if !D::CAN_WRITE_TO_DEVICE {
            return Err(ostd::Error::AccessDenied);
        }
        let mut writer = self.page.storage.writer();
        writer.skip(self.offset).limit(self.page.segment_size);
        Ok(writer)
    }

    pub fn sync_from_device(&self, byte_range: Range<usize>) -> Result<(), ostd::Error> {
        if byte_range.start > byte_range.end || byte_range.end > self.page.segment_size {
            return Err(ostd::Error::InvalidArgs);
        }
        self.page
            .dma_stream
            .sync_from_device(byte_range.start + self.offset..byte_range.end + self.offset)
            .unwrap();
        Ok(())
    }

    pub fn sync_to_device(&self, byte_range: Range<usize>) -> Result<(), ostd::Error> {
        if byte_range.start > byte_range.end || byte_range.end > self.page.segment_size {
            return Err(ostd::Error::InvalidArgs);
        }
        self.page
            .dma_stream
            .sync_to_device(byte_range.start + self.offset..byte_range.end + self.offset)
            .unwrap();
        Ok(())
    }
}

impl<D: DmaDirection> Drop for DmaSegment<D> {
    fn drop(&mut self) {
        let page = &self.page;
        let pool = page.pool.upgrade().unwrap();

        let mut avail_pages = pool.avail_pages.lock();

        let (became_avail, became_free) = {
            let mut allocated_segments = page.allocated_segments.lock();

            let nr_blocks_per_page = PAGE_SIZE / page.segment_size;
            let became_avail =
                get_next_free_index(&allocated_segments, nr_blocks_per_page).is_none();

            debug_assert!(self.offset < PAGE_SIZE);
            debug_assert!(self.offset.is_multiple_of(page.segment_size));
            let segment_idx = self.offset / page.segment_size;
            debug_assert_eq!(allocated_segments.get(segment_idx).unwrap(), true);
            allocated_segments.set(segment_idx, false);

            let became_free = allocated_segments.not_any();
            (became_avail, became_free)
        };

        if became_free && avail_pages.len() > pool.high_watermark {
            let mut all_pages = pool.all_pages.lock();
            avail_pages.retain(|page_| !Arc::ptr_eq(page_, page));
            all_pages.retain(|page_| !Arc::ptr_eq(page_, page));
            return;
        }

        if became_avail {
            avail_pages.push_back(page.clone());
        }
    }
}

#[cfg(ktest)]
mod test {
    use alloc::vec::Vec;

    use ostd::{
        mm::dma::{FromAndToDevice, ToDevice},
        prelude::*,
    };

    use super::*;

    #[ktest]
    fn alloc_page_size_segment() {
        let pool = DmaPool::<ToDevice>::new(PAGE_SIZE, 0, 100, false);
        let segments1: Vec<_> = (0..100)
            .map(|_| {
                let segment = pool.alloc_segment().unwrap();
                assert_eq!(segment.size(), PAGE_SIZE);
                assert!(segment.reader().is_err());
                assert!(segment.writer().is_ok());
                segment
            })
            .collect();

        assert_eq!(pool.num_pages(), 100);
        drop(segments1);
    }

    #[ktest]
    fn write_to_dma_segment() {
        let pool: Arc<DmaPool<ToDevice>> = DmaPool::new(PAGE_SIZE, 1, 2, false);
        let segment = pool.alloc_segment().unwrap();
        let mut writer = segment.writer().unwrap();
        let data = &[0u8, 1, 2, 3, 4] as &[u8];
        let size = writer.write(&mut VmReader::from(data));
        assert_eq!(size, data.len());
    }

    #[ktest]
    fn free_pool_pages() {
        let pool: Arc<DmaPool<ToDevice>> = DmaPool::new(PAGE_SIZE, 10, 50, false);
        let segments1: Vec<_> = (0..100)
            .map(|_| {
                let segment = pool.alloc_segment().unwrap();
                assert_eq!(segment.size(), PAGE_SIZE);
                assert!(segment.reader().is_err());
                assert!(segment.writer().is_ok());
                segment
            })
            .collect();
        assert_eq!(pool.num_pages(), 100);
        drop(segments1);
        assert_eq!(pool.num_pages(), 51);
    }

    #[ktest]
    fn alloc_small_size_segment() {
        const SEGMENT_SIZE: usize = PAGE_SIZE / 4;
        let pool: Arc<DmaPool<FromAndToDevice>> = DmaPool::new(SEGMENT_SIZE, 0, 10, false);
        let segments1: Vec<_> = (0..100)
            .map(|_| {
                let segment = pool.alloc_segment().unwrap();
                assert_eq!(segment.size(), PAGE_SIZE / 4);
                assert!(segment.reader().is_ok());
                assert!(segment.writer().is_ok());
                segment
            })
            .collect();

        assert_eq!(pool.num_pages(), 100 / 4);
        drop(segments1);
        assert_eq!(pool.num_pages(), 10);
    }

    #[ktest]
    fn read_dma_segments() {
        const SEGMENT_SIZE: usize = PAGE_SIZE / 4;
        let pool: Arc<DmaPool<FromAndToDevice>> = DmaPool::new(SEGMENT_SIZE, 1, 2, false);
        let segment = pool.alloc_segment().unwrap();
        assert_eq!(pool.num_pages(), 1);
        let mut writer = segment.writer().unwrap();
        let data = &[0u8, 1, 2, 3, 4] as &[u8];
        let size = writer.write(&mut VmReader::from(data));
        assert_eq!(size, data.len());

        let mut read_buf = [0u8; 5];
        let mut reader = segment.reader().unwrap();
        reader.read(&mut VmWriter::from(&mut read_buf as &mut [u8]));
        assert_eq!(&read_buf, data);
    }
}
