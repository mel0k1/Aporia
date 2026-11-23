use core::{
    ops::{Deref, Range},
    sync::atomic::Ordering,
};

use align_ext::AlignExt;
use aster_block::{
    BlockDevice,
    bio::{BioCompleteFn, BioDirection, BioSegment, BioStatus},
    id::Bid,
};
use io_util::batch::IoBatch;
use ostd::mm::{Segment, VmIo, VmIoFill};

use crate::prelude::*;

mod cache_page;
#[cfg(ktest)]
mod tests;
mod vmo;

pub(crate) use cache_page::{CachePage, CachePageExt, CachePageMeta, LockedCachePage};
pub use vmo::Vmo;
pub(crate) use vmo::{VmoCommitError, VmoFlags, VmoMapMode, VmoOptions};

#[derive(Debug)]
pub(crate) struct PageCache(Arc<Vmo>);

impl PageCache {
    pub(crate) fn new_with_backend(
        size: usize,
        backend: Weak<dyn PageCacheBackend>,
    ) -> Result<Self> {
        Ok(Self(VmoOptions::new_page_cache(size, backend).alloc()?))
    }

    pub(crate) fn new_anon(size: usize) -> Result<Self> {
        Ok(Self(VmoOptions::new_anon(size).alloc()?))
    }

    pub(crate) fn as_vmo(&self) -> &Arc<Vmo> {
        &self.0
    }

    pub(crate) fn size(&self) -> usize {
        self.0.size()
    }

    pub(crate) fn resize(&mut self, new_file_size: usize, old_file_size: usize) -> Result<()> {
        let vmo = &self.0;
        assert!(vmo.flags.contains(VmoFlags::RESIZABLE));

        if new_file_size < old_file_size && !new_file_size.is_multiple_of(PAGE_SIZE) {
            let fill_zero_end = old_file_size.min(new_file_size.align_up(PAGE_SIZE));
            self.fill_zeros(new_file_size..fill_zero_end)?;
        } else if new_file_size > old_file_size && !old_file_size.is_multiple_of(PAGE_SIZE) {
            let fill_zero_end = new_file_size.min(old_file_size.align_up(PAGE_SIZE));
            self.fill_zeros(old_file_size..fill_zero_end)?;
        }

        let rmap = if new_file_size < old_file_size {
            Some(vmo.rmap.lock())
        } else {
            None
        };

        let new_cache_size = new_file_size.align_up(PAGE_SIZE);
        let locked_pages = vmo.pages.lock();
        let old_cache_size = vmo.size();
        if new_cache_size == old_cache_size {
            return Ok(());
        }

        vmo.size.store(new_cache_size, Ordering::Release);

        if let Some(locked_rmap) = rmap {
            let decommit_range = new_cache_size..old_cache_size;
            if let Some(backed_vmo) = vmo.as_backed_vmo() {
                backed_vmo.decommit_pages(locked_rmap, locked_pages, &decommit_range)?;
            } else {
                vmo.decommit_anon_pages(locked_rmap, locked_pages, decommit_range)?;
            }
        }

        Ok(())
    }

    pub(crate) fn flush_range(&self, range: Range<usize>) -> Result<()> {
        let Some(vmo) = self.0.as_backed_vmo() else {
            return Ok(());
        };

        vmo.flush_dirty_pages(&range)
    }

    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(crate) fn evict_range(&self, range: Range<usize>) -> Result<()> {
        let Some(vmo) = self.0.as_backed_vmo() else {
            return Ok(());
        };

        vmo.evict_up_to_date_pages(&range)
    }

    pub(crate) fn invalidate_range(&self, range: Range<usize>) -> Result<()> {
        let Some(vmo) = self.0.as_backed_vmo() else {
            return Ok(());
        };

        vmo.flush_dirty_pages(&range)?;
        vmo.evict_up_to_date_pages(&range)
    }

    pub(crate) fn fill_zeros(&self, range: Range<usize>) -> Result<()> {
        self.0.fill_zeros(range)
    }
}

impl VmIo for PageCache {
    fn read(&self, offset: usize, writer: &mut VmWriter) -> ostd::Result<()> {
        self.0.read(offset, writer)?;
        Ok(())
    }

    fn write(&self, offset: usize, reader: &mut VmReader) -> ostd::Result<()> {
        self.0.write(offset, reader)?;
        Ok(())
    }
}

pub(crate) trait PageCacheBackend: Sync + Send {
    fn read_page_async(
        &self,
        idx: usize,
        locked_page: LockedCachePage,
        io_batch: &mut IoBatch,
    ) -> Result<()>;

    #[expect(dead_code)]
    fn read_pages_async(&self, pages: PageRun<'_>, io_batch: &mut IoBatch) -> Result<()> {
        for (idx, locked_page) in pages.into_indexed() {
            self.read_page_async(idx, locked_page, io_batch)?;
        }
        Ok(())
    }

    fn write_page_async(
        &self,
        idx: usize,
        locked_page: LockedCachePage,
        io_batch: &mut IoBatch,
    ) -> Result<()>;

    fn write_pages_async(&self, pages: PageRun<'_>, io_batch: &mut IoBatch) -> Result<()> {
        for (idx, locked_page) in pages.into_indexed() {
            self.write_page_async(idx, locked_page, io_batch)?;
        }
        Ok(())
    }
}

impl dyn PageCacheBackend {
    pub(crate) fn read_page(&self, idx: usize, page: LockedCachePage) -> Result<()> {
        let mut io_batch = IoBatch::with_capacity(1);
        self.read_page_async(idx, page, &mut io_batch)?;
        io_batch.wait_all()?;
        Ok(())
    }
}

pub(crate) struct PageRun<'a> {
    start_idx: usize,
    pages: &'a mut dyn ExactSizeIterator<Item = LockedCachePage>,
}

impl Iterator for PageRun<'_> {
    type Item = LockedCachePage;

    fn next(&mut self) -> Option<Self::Item> {
        let page = self.pages.next()?;
        self.start_idx += 1;
        Some(page)
    }
}

impl<'a> PageRun<'a> {
    pub(super) fn new<I>(start_idx: usize, pages: &'a mut I) -> Self
    where
        I: ExactSizeIterator<Item = LockedCachePage>,
    {
        debug_assert!(pages.len() > 0);
        Self { start_idx, pages }
    }

    pub(crate) fn start_idx(&self) -> usize {
        self.start_idx
    }

    pub(crate) fn len(&self) -> usize {
        self.pages.len()
    }

    fn into_indexed(self) -> impl ExactSizeIterator<Item = (usize, LockedCachePage)> {
        let start_idx = self.start_idx;
        self.pages
            .enumerate()
            .map(move |(offset, page)| (start_idx + offset, page))
    }

    pub(crate) fn submit_contiguous_read_pages(
        &mut self,
        block_device: &dyn BlockDevice,
        start_bid: Bid,
        len: usize,
        io_batch: &mut IoBatch,
    ) -> Result<()> {
        if self.len() < len {
            return Err(errno!(EINVAL, "page run is shorter than len"));
        }

        let max_segments = block_device.metadata().max_nr_segments_per_bio.max(1);
        let mut start_bid = start_bid;
        let mut remaining_pages = len;

        while remaining_pages > 0 {
            let nr_pages = max_segments.min(remaining_pages);
            let mut bio_segments = Vec::with_capacity(nr_pages);
            let mut complete_fns = Vec::with_capacity(nr_pages);
            for locked_page in self.by_ref().take(nr_pages) {
                let (bio_segment, complete_fn) = prepare_read_page(locked_page);
                bio_segments.push(bio_segment);
                complete_fns.push(complete_fn);
            }

            let complete_fn = Box::new(move |status| {
                for complete_fn in complete_fns {
                    complete_fn(status);
                }
            });
            block_device.read_blocks_async(start_bid, bio_segments, Some(complete_fn), io_batch)?;
            start_bid = start_bid + nr_pages as u64;
            remaining_pages -= nr_pages;
        }

        Ok(())
    }

    pub(crate) fn submit_contiguous_write_pages(
        &mut self,
        block_device: &dyn BlockDevice,
        start_bid: Bid,
        len: usize,
        io_batch: &mut IoBatch,
    ) -> Result<()> {
        if self.len() < len {
            return Err(errno!(EINVAL, "page run is shorter than len"));
        }

        let max_segments = block_device.metadata().max_nr_segments_per_bio.max(1);
        let mut start_bid = start_bid;
        let mut remaining_pages = len;

        while remaining_pages > 0 {
            let nr_pages = max_segments.min(remaining_pages);
            let mut bio_segments = Vec::with_capacity(nr_pages);
            let mut pages = Vec::with_capacity(nr_pages);
            for locked_page in self.by_ref().take(nr_pages) {
                let (bio_segment, page) = prepare_write_page_segment(locked_page);
                bio_segments.push(bio_segment);
                pages.push(page);
            }

            let pages = Arc::new(pages);
            let completion_pages = pages.clone();
            let complete_fn = Box::new(move |status| {
                for page in completion_pages.iter() {
                    complete_write_page(page, status);
                }
            });
            let result = block_device.write_blocks_async(
                start_bid,
                bio_segments,
                Some(complete_fn),
                io_batch,
            );
            if let Err(err) = result {
                for page in pages.iter() {
                    let locked_page = page.lock_guard();
                    locked_page.set_dirty();
                    locked_page.clear_writing_back();
                }
                return Err(err.into());
            }
            start_bid = start_bid + nr_pages as u64;
            remaining_pages -= nr_pages;
        }

        Ok(())
    }
}

pub(crate) trait BlockAsPageCacheBackend: Sync + Send {
    fn submit_read_bio(
        &self,
        idx: usize,
        bio_segment: BioSegment,
        complete_fn: BioCompleteFn,
        io_batch: &mut IoBatch,
    ) -> Result<()>;

    fn submit_read_page_run(&self, pages: PageRun<'_>, io_batch: &mut IoBatch) -> Result<()> {
        for (idx, locked_page) in pages.into_indexed() {
            let (bio_segment, complete_fn) = prepare_read_page(locked_page);
            self.submit_read_bio(idx, bio_segment, complete_fn, io_batch)?;
        }
        Ok(())
    }

    fn submit_write_bio(
        &self,
        idx: usize,
        bio_segment: BioSegment,
        complete_fn: BioCompleteFn,
        io_batch: &mut IoBatch,
    ) -> Result<()>;

    fn submit_write_page_run(&self, pages: PageRun<'_>, io_batch: &mut IoBatch) -> Result<()> {
        for (idx, locked_page) in pages.into_indexed() {
            let rollback_page = locked_page.clone();
            let (bio_segment, complete_fn) = prepare_write_page(locked_page);
            let res = self.submit_write_bio(idx, bio_segment, complete_fn, io_batch);
            if res.is_err() {
                let locked_page = rollback_page.lock();
                locked_page.set_dirty();
                locked_page.clear_writing_back();
            }
            res?;
        }
        Ok(())
    }
}

impl<T: BlockAsPageCacheBackend> PageCacheBackend for T {
    fn read_page_async(
        &self,
        idx: usize,
        locked_page: LockedCachePage,
        io_batch: &mut IoBatch,
    ) -> Result<()> {
        let (bio_segment, complete_fn) = prepare_read_page(locked_page);
        self.submit_read_bio(idx, bio_segment, complete_fn, io_batch)
    }

    fn read_pages_async(&self, pages: PageRun<'_>, io_batch: &mut IoBatch) -> Result<()> {
        self.submit_read_page_run(pages, io_batch)
    }

    fn write_page_async(
        &self,
        idx: usize,
        locked_page: LockedCachePage,
        io_batch: &mut IoBatch,
    ) -> Result<()> {
        let rollback_page = locked_page.clone();

        let (bio_segment, complete_fn) = prepare_write_page(locked_page);
        let res = self.submit_write_bio(idx, bio_segment, complete_fn, io_batch);
        if res.is_err() {
            let locked_page = rollback_page.lock();
            locked_page.set_dirty();
            locked_page.clear_writing_back();
        }

        res
    }

    fn write_pages_async(&self, pages: PageRun<'_>, io_batch: &mut IoBatch) -> Result<()> {
        self.submit_write_page_run(pages, io_batch)
    }
}

fn prepare_read_page(locked_page: LockedCachePage) -> (BioSegment, BioCompleteFn) {
    let bio_segment = BioSegment::new_from_segment(
        Segment::from(locked_page.deref().clone()).into(),
        BioDirection::FromDevice,
    );

    let complete_fn = Box::new(move |status| {
        if status == BioStatus::Zeros {
            locked_page.fill_zeros(0, PAGE_SIZE).unwrap();
            locked_page.set_up_to_date();
        } else if status == BioStatus::Complete {
            locked_page.set_up_to_date();
        }
    });
    (bio_segment, complete_fn)
}

fn prepare_write_page(locked_page: LockedCachePage) -> (BioSegment, BioCompleteFn) {
    let (bio_segment, page) = prepare_write_page_segment(locked_page);
    let complete_fn = Box::new(move |status| complete_write_page(&page, status));
    (bio_segment, complete_fn)
}

fn prepare_write_page_segment(locked_page: LockedCachePage) -> (BioSegment, CachePage) {
    locked_page.wait_until_finish_writing_back();

    let bio_segment = BioSegment::new_from_segment(
        Segment::from(locked_page.deref().clone()).into(),
        BioDirection::ToDevice,
    );

    locked_page.set_writing_back();
    locked_page.set_up_to_date();

    let page = locked_page.unlock();
    (bio_segment, page)
}

fn complete_write_page(page: &CachePage, status: BioStatus) {
    page.clear_writing_back();
    if status != BioStatus::Complete {
        ostd::error!("writeback I/O failed with status {status:?}; data may be lost");
    }
}
