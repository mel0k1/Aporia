use core::{
    cmp::min,
    ops::{Deref, Range},
    sync::atomic::{AtomicIsize, AtomicUsize, Ordering},
};

use align_ext::AlignExt;
use io_util::batch::IoBatch;
use ostd::{
    mm::{HasPaddr, io::util::HasVmReaderWriter},
    task::disable_preempt,
};
use xarray::{Cursor, LockedXArray, XArray};

use crate::{
    prelude::*,
    vm::{
        page_cache::{CachePage, CachePageExt, PageCacheBackend, PageRun, cache_page::PageState},
        vmar::Rmap,
    },
};

mod options;

pub(crate) use options::VmoOptions;

pub struct Vmo {
    pub(super) backend: Option<Weak<dyn PageCacheBackend>>,
    pub(super) flags: VmoFlags,
    pub(super) pages: XArray<CachePage>,
    pub(super) size: AtomicUsize,
    pub(super) writable_mapping_status: WritableMappingStatus,
    pub(super) rmap: Mutex<Rmap>,
}

impl Debug for Vmo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Vmo")
            .field("has_backend", &self.backend.is_some())
            .field("flags", &self.flags)
            .field("size", &self.size)
            .field("writable_mapping_status", &self.writable_mapping_status)
            .finish_non_exhaustive()
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub(crate) struct VmoFlags: u32 {
        const RESIZABLE  = 1 << 0;
        const CONTIGUOUS = 1 << 1;
        const DMA        = 1 << 2;
    }
}

#[derive(Debug)]
pub(crate) enum VmoCommitError {
    Err(Error),
    NeedIo { index: usize },
    WaitUntilInit { index: usize, page: CachePage },
}

impl VmoCommitError {
    pub(crate) fn pending_index(&self) -> Result<usize> {
        match self {
            Self::NeedIo { index } | Self::WaitUntilInit { index, .. } => Ok(*index),
            Self::Err(e) => Err(*e),
        }
    }
}

impl From<Error> for VmoCommitError {
    fn from(e: Error) -> Self {
        VmoCommitError::Err(e)
    }
}

impl From<ostd::Error> for VmoCommitError {
    #[track_caller]
    fn from(e: ostd::Error) -> Self {
        Error::from(e).into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommitMode {
    Read,
    Overwrite,
}

impl CommitMode {
    fn skips_backend_read(self) -> bool {
        matches!(self, Self::Overwrite)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VmoMapMode {
    Private,
    SharedRead,
    SharedWrite,
}

impl VmoMapMode {
    fn ensure(self, page: CachePage) {
        match page.state() {
            PageState::UpToDate if self.is_write() => {
                let locked_page = page.lock_guard();
                if locked_page.state().is_up_to_date() {
                    locked_page.set_dirty();
                }
            }
            PageState::UpToDate | PageState::Dirty => (),
            PageState::Evicted if !self.is_shared() => (),
            state @ (PageState::Uninit | PageState::Evicted) => {
                debug_assert_eq!(state, PageState::Evicted);
                let _ = page.lock_guard();
            }
        }
    }

    fn try_ensure(self, page_idx: usize, page: &CachePage) -> Result<Self, VmoCommitError> {
        match page.state() {
            PageState::UpToDate if self.is_write() => {
                if let Some(locked_page) = page.try_lock_guard()
                    && locked_page.state().is_up_to_date()
                {
                    locked_page.set_dirty();
                    Ok(Self::SharedWrite)
                } else {
                    Err(VmoCommitError::NeedIo { index: page_idx })
                }
            }
            PageState::UpToDate => Ok(Self::SharedRead),
            PageState::Dirty => Ok(Self::SharedWrite),
            PageState::Evicted if !self.is_shared() => Ok(Self::Private),
            PageState::Uninit | PageState::Evicted => {
                Err(VmoCommitError::NeedIo { index: page_idx })
            }
        }
    }

    fn is_write(self) -> bool {
        matches!(self, Self::SharedWrite)
    }

    fn is_shared(self) -> bool {
        matches!(self, Self::SharedRead | Self::SharedWrite)
    }
}

impl Vmo {
    pub(crate) fn commit_on(&self, page_idx: usize, map_mode: VmoMapMode) -> Result<()> {
        let page = self.commit_on_internal(page_idx, CommitMode::Read)?;
        map_mode.ensure(page);
        Ok(())
    }

    fn commit_on_internal(&self, page_idx: usize, commit_mode: CommitMode) -> Result<CachePage> {
        if let Some(backed_vmo) = self.as_backed_vmo() {
            return backed_vmo.commit_on_internal(page_idx, commit_mode);
        }

        self.commit_on_anonymous(page_idx)
    }

    fn commit_on_anonymous(&self, page_idx: usize) -> Result<CachePage> {
        let mut locked_pages = self.pages.lock();
        if page_idx >= self.size().div_ceil(PAGE_SIZE) {
            return Err(errno!(EINVAL, "the offset is outside the VMO"));
        }

        let mut cursor = locked_pages.cursor_mut(page_idx as u64);
        if let Some(page) = cursor.load() {
            return Ok(page.clone());
        }

        let new_page = CachePage::alloc_zero()?;
        cursor.store(new_page.clone());
        Ok(new_page)
    }

    pub(crate) fn try_commit_page(
        &self,
        offset: usize,
        map_mode: VmoMapMode,
    ) -> Result<(CachePage, VmoMapMode), VmoCommitError> {
        let page_idx = offset / PAGE_SIZE;
        if offset >= self.size() {
            return Err(VmoCommitError::Err(errno!(
                EINVAL,
                "the offset is outside the VMO",
            )));
        }

        let (_, page) = {
            let guard = disable_preempt();
            let mut cursor = self.pages.cursor(&guard, page_idx as u64);
            self.try_commit_with_cursor(&mut cursor, CommitMode::Overwrite)?
        };
        let mode = map_mode.try_ensure(page_idx, &page)?;
        Ok((page, mode))
    }

    fn try_commit_with_cursor(
        &self,
        cursor: &mut Cursor<'_, CachePage>,
        commit_mode: CommitMode,
    ) -> Result<(usize, CachePage), VmoCommitError> {
        if let Some(backed_vmo) = self.as_backed_vmo() {
            return backed_vmo.try_commit_with_cursor(cursor, commit_mode);
        } else if let Some(page) = cursor.load() {
            let index = cursor.index() as usize;
            return Ok((index, page.clone()));
        }

        let index = cursor.index() as usize;
        let page = self.commit_on_anonymous(index)?;
        Ok((index, page))
    }

    pub(crate) fn try_operate_on_range<F>(
        &self,
        range: &Range<usize>,
        mut operate: F,
        map_mode: VmoMapMode,
    ) -> Result<(), VmoCommitError>
    where
        F: FnMut(
            &mut dyn FnMut() -> Result<(usize, CachePage), VmoCommitError>,
        ) -> Result<(), VmoCommitError>,
    {
        self.try_operate_on_range_internal(
            range,
            |commit_fn| {
                let mut new_commit_fn = || {
                    let (idx, page) = commit_fn()?;
                    map_mode.try_ensure(idx, &page)?;
                    Ok((idx, page))
                };
                operate(&mut new_commit_fn)
            },
            CommitMode::Overwrite,
        )
    }

    fn try_operate_on_range_internal<F>(
        &self,
        range: &Range<usize>,
        mut operate: F,
        commit_mode: CommitMode,
    ) -> Result<(), VmoCommitError>
    where
        F: FnMut(
            &mut dyn FnMut() -> Result<(usize, CachePage), VmoCommitError>,
        ) -> Result<(), VmoCommitError>,
    {
        if range.end > self.size() {
            return Err(VmoCommitError::Err(errno!(
                EINVAL,
                "the range to operate exceeds the VMO size",
            )));
        }

        let page_idx_range = get_page_idx_range(range);
        let guard = disable_preempt();
        let mut cursor = self.pages.cursor(&guard, page_idx_range.start as u64);
        for _ in page_idx_range {
            let mut commit_fn = || self.try_commit_with_cursor(&mut cursor, commit_mode);
            operate(&mut commit_fn)?;
            cursor.next();
        }
        Ok(())
    }

    pub(crate) fn size(&self) -> usize {
        self.size.load(Ordering::Acquire)
    }

    pub(crate) fn writable_mapping_status(&self) -> &WritableMappingStatus {
        debug_assert!(!self.has_backend());
        &self.writable_mapping_status
    }

    pub(crate) fn rmap(&self) -> &Mutex<Rmap> {
        &self.rmap
    }

    pub(super) fn decommit_anon_pages(
        &self,
        mut locked_rmap: MutexGuard<Rmap>,
        mut locked_pages: LockedXArray<CachePage>,
        range: Range<usize>,
    ) -> Result<()> {
        debug_assert!(!self.has_backend());

        let page_idx_range = get_page_idx_range(&range);
        let mut cursor = locked_pages.cursor_mut(page_idx_range.start as u64);

        loop {
            cursor.remove();
            let page_idx = cursor.next_present();
            if page_idx.is_none_or(|idx| idx >= page_idx_range.end as u64) {
                break;
            }
        }

        drop(locked_pages);

        locked_rmap.unmap(page_idx_range.start * PAGE_SIZE..page_idx_range.end * PAGE_SIZE);

        Ok(())
    }

    pub(super) fn as_backed_vmo(&self) -> Option<BackedVmo<'_>> {
        self.backend.as_ref().and_then(|weak_backend| {
            weak_backend
                .upgrade()
                .map(|backend| BackedVmo { vmo: self, backend })
        })
    }

    fn has_backend(&self) -> bool {
        self.backend.is_some()
    }
}

impl Vmo {
    const PAGE_BATCH_CAPACITY: usize = 32;

    pub(crate) fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<()> {
        let read_len = writer.avail().min(self.size().saturating_sub(offset));
        if read_len == 0 {
            return Ok(());
        }
        let read_end = offset + read_len;

        let range = offset..read_end;
        let page_idx_range = get_page_idx_range(&range);
        let mut current_idx = page_idx_range.start;
        let mut page_offset = offset % PAGE_SIZE;
        let mut page_batch =
            Vec::with_capacity(min(page_idx_range.len(), Self::PAGE_BATCH_CAPACITY));

        while current_idx < page_idx_range.end {
            self.collect_pages(
                current_idx,
                page_idx_range.end,
                CommitMode::Read,
                &mut page_batch,
            )?;

            for (_, page) in page_batch.iter() {
                page.reader().skip(page_offset).read_fallible(writer)?;
                page_offset = 0;
            }

            current_idx = page_batch.last().unwrap().0 + 1;
        }

        Ok(())
    }

    pub(crate) fn write(&self, offset: usize, reader: &mut VmReader) -> Result<()> {
        let write_len = reader.remain().min(self.size().saturating_sub(offset));
        if write_len == 0 {
            return Ok(());
        }
        let write_end = offset + write_len;

        let write_range = offset..write_end;
        let mut page_offset = offset % PAGE_SIZE;
        let mut page_batch = Vec::with_capacity(min(
            write_range.len().div_ceil(PAGE_SIZE),
            Self::PAGE_BATCH_CAPACITY,
        ));

        if !self.has_backend() {
            return self.write_anonymous_pages(
                &write_range,
                &mut page_offset,
                reader,
                &mut page_batch,
            );
        }

        if write_range.len() < PAGE_SIZE {
            return self.write_pages_with_backend(
                &write_range,
                &mut page_offset,
                reader,
                CommitMode::Read,
                &mut page_batch,
            );
        }

        let up_align_start = write_range.start.align_up(PAGE_SIZE);
        let down_align_end = write_range.end.align_down(PAGE_SIZE);

        if write_range.start != up_align_start {
            let head = write_range.start..up_align_start;
            self.write_pages_with_backend(
                &head,
                &mut page_offset,
                reader,
                CommitMode::Read,
                &mut page_batch,
            )?;
        }
        if up_align_start != down_align_end {
            let mid = up_align_start..down_align_end;
            self.write_pages_with_backend(
                &mid,
                &mut page_offset,
                reader,
                CommitMode::Overwrite,
                &mut page_batch,
            )?;
        }
        if down_align_end != write_range.end {
            let tail = down_align_end..write_range.end;
            self.write_pages_with_backend(
                &tail,
                &mut page_offset,
                reader,
                CommitMode::Read,
                &mut page_batch,
            )?;
        }

        Ok(())
    }

    pub(super) fn fill_zeros(&self, range: Range<usize>) -> Result<()> {
        if range.is_empty() {
            return Ok(());
        }
        if range.end > self.size() {
            return Err(errno!(EINVAL, "the range to fill exceeds the VMO size"));
        }

        static ZERO_PAGE: [u8; PAGE_SIZE] = [0; PAGE_SIZE];

        let mut current_offset = range.start;
        while current_offset < range.end {
            let page_remaining = PAGE_SIZE - current_offset % PAGE_SIZE;
            let write_len = (range.end - current_offset).min(page_remaining);
            let mut reader = VmReader::from(&ZERO_PAGE[..write_len]).to_fallible();

            self.write(current_offset, &mut reader)?;
            current_offset += write_len;
        }

        Ok(())
    }

    fn write_anonymous_pages(
        &self,
        range: &Range<usize>,
        page_offset: &mut usize,
        reader: &mut VmReader,
        page_batch: &mut Vec<(usize, CachePage)>,
    ) -> Result<()> {
        let page_idx_range = get_page_idx_range(range);
        let mut current_idx = page_idx_range.start;

        while current_idx < page_idx_range.end {
            self.collect_pages(
                current_idx,
                page_idx_range.end,
                CommitMode::Read,
                page_batch,
            )?;

            for (_, page) in page_batch.iter() {
                page.writer().skip(*page_offset).write_fallible(reader)?;
                *page_offset = 0;
            }

            current_idx = page_batch.last().unwrap().0 + 1;
        }

        Ok(())
    }

    fn write_pages_with_backend(
        &self,
        range: &Range<usize>,
        page_offset: &mut usize,
        reader: &mut VmReader,
        commit_mode: CommitMode,
        page_batch: &mut Vec<(usize, CachePage)>,
    ) -> Result<()> {
        let page_idx_range = get_page_idx_range(range);
        let mut current_idx = page_idx_range.start;

        while current_idx < page_idx_range.end {
            self.collect_pages(current_idx, page_idx_range.end, commit_mode, page_batch)?;

            let mut next_idx = page_batch.last().unwrap().0 + 1;

            for (idx, page) in page_batch.drain(..) {
                let locked_page = page.lock();

                let state = locked_page.state();
                if state.is_evicted() {
                    next_idx = idx;
                    continue;
                }

                let written_size = match locked_page
                    .writer()
                    .skip(*page_offset)
                    .write_fallible(reader)
                {
                    Ok(written_size) => written_size,
                    Err((err, written_size)) => {
                        if written_size > 0 && state.is_up_to_date() {
                            locked_page.set_dirty();
                        }
                        return Err(Error::from(err));
                    }
                };

                if written_size > 0 {
                    locked_page.set_dirty();
                }

                *page_offset = 0;
            }

            current_idx = next_idx;
        }

        Ok(())
    }

    fn collect_pages(
        &self,
        start_idx: usize,
        end_idx: usize,
        commit_mode: CommitMode,
        pages: &mut Vec<(usize, CachePage)>,
    ) -> Result<()> {
        let end_idx = end_idx.min(start_idx + pages.capacity());
        pages.clear();
        let range = (start_idx * PAGE_SIZE)..(end_idx * PAGE_SIZE);
        let mut current_range = range.clone();

        'retry: loop {
            let mut operate =
                |commit_fn: &mut dyn FnMut() -> Result<(usize, CachePage), VmoCommitError>| {
                    let (idx, page) = commit_fn()?;
                    pages.push((idx, page));
                    Ok(())
                };
            match self.try_operate_on_range_internal(&current_range, &mut operate, commit_mode) {
                Ok(()) => break 'retry,
                Err(err) => {
                    let (idx, page) = self.handle_commit_error(err, commit_mode)?;
                    pages.push((idx, page));
                    current_range.start = (idx + 1) * PAGE_SIZE;
                    if current_range.start >= range.end {
                        break 'retry;
                    }
                }
            }
        }

        Ok(())
    }

    fn handle_commit_error(
        &self,
        err: VmoCommitError,
        commit_mode: CommitMode,
    ) -> Result<(usize, CachePage)> {
        match err {
            VmoCommitError::Err(e) => Err(e),
            VmoCommitError::NeedIo { index } => {
                Ok((index, self.commit_on_internal(index, commit_mode)?))
            }
            VmoCommitError::WaitUntilInit { index, page } => {
                page.ensure_init(|locked_page| {
                    self.as_backed_vmo()
                        .unwrap()
                        .backend
                        .read_page(index, locked_page)
                })?;
                Ok((index, page))
            }
        }
    }
}

pub(crate) struct BackedVmo<'a> {
    vmo: &'a Vmo,
    backend: Arc<dyn PageCacheBackend>,
}

enum PageSelection {
    Flush,
    Evict,
}

impl PageSelection {
    fn should_collect(&self, page: &CachePage) -> bool {
        let state = page.state();
        match self {
            PageSelection::Flush if state.is_dirty() => true,
            PageSelection::Flush if state.is_uninit() => false,
            PageSelection::Flush if let Some(locked_page) = page.try_lock_guard() => {
                locked_page.is_writing_back()
            }
            PageSelection::Flush => true,
            PageSelection::Evict => state.is_up_to_date(),
        }
    }
}

impl<'a> BackedVmo<'a> {
    pub(super) fn flush_dirty_pages(&self, range: &Range<usize>) -> Result<()> {
        if range.start >= self.size() {
            return Ok(());
        }

        let mut locked_rmap = self.rmap.lock();

        let page_idx_range = get_page_idx_range(range);
        let pages_to_flush = self.collect_pages_if(&page_idx_range, PageSelection::Flush);
        if pages_to_flush.is_empty() {
            return Ok(());
        }

        let mut locked_dirty_pages = Vec::new();
        let mut maybe_writing_back_pages = Vec::new();
        for (idx, page) in pages_to_flush {
            if !page.state().is_dirty() {
                maybe_writing_back_pages.push(page);
                continue;
            }
            let locked_page = page.lock();
            if locked_page.state().is_dirty() {
                locked_page.set_up_to_date();
                locked_dirty_pages.push((idx, locked_page));
            } else if locked_page.is_writing_back() {
                maybe_writing_back_pages.push(locked_page.unlock());
            }
        }

        locked_rmap.freeze(page_idx_range.start * PAGE_SIZE..page_idx_range.end * PAGE_SIZE);

        drop(locked_rmap);

        let mut io_batch = IoBatch::with_capacity(locked_dirty_pages.len());
        for locked_page_run in locked_dirty_pages
            .chunk_by_mut(|(page_idx, _), (next_page_idx, _)| page_idx + 1 == *next_page_idx)
        {
            if locked_page_run.len() == 1 {
                let (idx, locked_page) = &mut locked_page_run[0];
                self.backend
                    .write_page_async(*idx, locked_page.detach(), &mut io_batch)?;
                continue;
            }

            let start_idx = locked_page_run[0].0;
            let mut locked_pages = locked_page_run
                .iter_mut()
                .map(|(_, locked_page)| locked_page.detach());
            let pages = PageRun::new(start_idx, &mut locked_pages);
            if let Err(err) = self.backend.write_pages_async(pages, &mut io_batch) {
                for locked_page in locked_pages {
                    locked_page.set_dirty();
                }
                let _ = io_batch.wait_all();
                return Err(err);
            }
        }
        io_batch.wait_all()?;

        for page in maybe_writing_back_pages {
            let locked_page = page.lock();
            locked_page.wait_until_finish_writing_back();
        }

        Ok(())
    }

    pub(super) fn evict_up_to_date_pages(&self, range: &Range<usize>) -> Result<()> {
        if range.start >= self.size() {
            return Ok(());
        }

        let mut locked_rmap = self.rmap.lock();

        let page_idx_range = get_page_idx_range(range);
        let up_to_date_pages = self.collect_pages_if(&page_idx_range, PageSelection::Evict);
        if up_to_date_pages.is_empty() {
            return Ok(());
        }

        let locked_up_to_date_pages = up_to_date_pages
            .into_iter()
            .filter_map(|(idx, page)| {
                let locked_page = page.lock();
                if !locked_page.state().is_up_to_date() {
                    return None;
                }
                locked_page.set_evicted();
                Some((idx, locked_page))
            })
            .collect::<Vec<_>>();
        if locked_up_to_date_pages.is_empty() {
            return Ok(());
        }

        locked_rmap.unmap(page_idx_range.start * PAGE_SIZE..page_idx_range.end * PAGE_SIZE);

        drop(locked_rmap);

        for (idx, page) in locked_up_to_date_pages {
            page.wait_until_finish_writing_back();
            let mut locked_pages = self.pages.lock();
            let mut cursor_mut = locked_pages.cursor_mut(idx as u64);
            let removed_page = cursor_mut.remove();
            debug_assert_eq!(removed_page.unwrap().paddr(), page.paddr());
        }

        Ok(())
    }

    pub(super) fn decommit_pages(
        &self,
        mut locked_rmap: MutexGuard<Rmap>,
        locked_pages: LockedXArray<CachePage>,
        range: &Range<usize>,
    ) -> Result<()> {
        let page_idx_range = get_page_idx_range(range);
        let removed_pages = self.remove_pages_in(locked_pages, &page_idx_range);
        if removed_pages.is_empty() {
            return Ok(());
        }

        locked_rmap.unmap(page_idx_range.start * PAGE_SIZE..page_idx_range.end * PAGE_SIZE);

        drop(locked_rmap);

        for (_, page) in removed_pages {
            let locked_page = page.lock();
            locked_page.wait_until_finish_writing_back();
        }

        Ok(())
    }

    fn commit_on_internal(&self, page_idx: usize, commit_mode: CommitMode) -> Result<CachePage> {
        let mut locked_pages = self.pages.lock();
        if page_idx >= self.size().div_ceil(PAGE_SIZE) {
            return Err(errno!(EINVAL, "the offset is outside the VMO"));
        }

        let mut cursor = locked_pages.cursor_mut(page_idx as u64);

        if let Some(page) = cursor.load() {
            let page = page.clone();
            drop(locked_pages);

            if !commit_mode.skips_backend_read() {
                page.ensure_init(|locked_page| self.backend.read_page(page_idx, locked_page))?;
                return Ok(page);
            }

            return Ok(page);
        };

        let uninit_page = CachePage::alloc_uninit()?;
        cursor.store(uninit_page.clone());
        drop(locked_pages);

        if commit_mode.skips_backend_read() {
            Ok(uninit_page)
        } else {
            uninit_page.ensure_init(|locked_page| self.backend.read_page(page_idx, locked_page))?;
            Ok(uninit_page)
        }
    }

    fn try_commit_with_cursor(
        &self,
        cursor: &mut Cursor<'_, CachePage>,
        commit_mode: CommitMode,
    ) -> Result<(usize, CachePage), VmoCommitError> {
        let page_idx = cursor.index() as usize;

        let Some(page) = cursor.load() else {
            return Err(VmoCommitError::NeedIo { index: page_idx });
        };

        if !commit_mode.skips_backend_read() && page.state().is_uninit() {
            return Err(VmoCommitError::WaitUntilInit {
                index: page_idx,
                page: page.clone(),
            });
        }

        Ok((page_idx, page.clone()))
    }

    fn collect_pages_if(
        &self,
        page_idx_range: &Range<usize>,
        cond: PageSelection,
    ) -> Vec<(usize, CachePage)> {
        if page_idx_range.is_empty() {
            return Vec::new();
        }

        let mut collected_pages = Vec::new();

        let preempt_guard = disable_preempt();
        let mut cursor = self
            .pages
            .cursor(&preempt_guard, page_idx_range.start as u64);
        let mut index = page_idx_range.start;

        loop {
            if let Some(page) = cursor.load()
                && cond.should_collect(&page)
            {
                collected_pages.push((index, page.clone()));
            }

            let Some(next_index) = cursor.next_present() else {
                break;
            };
            index = next_index as usize;
            if index >= page_idx_range.end {
                break;
            }
        }

        collected_pages
    }

    fn remove_pages_in(
        &self,
        mut locked_pages: LockedXArray<CachePage>,
        page_idx_range: &Range<usize>,
    ) -> Vec<(usize, CachePage)> {
        if page_idx_range.is_empty() {
            return Vec::new();
        }

        let mut removed_pages = Vec::new();

        let mut cursor_mut = locked_pages.cursor_mut(page_idx_range.start as u64);
        let mut index = page_idx_range.start;

        loop {
            if let Some(page) = cursor_mut.remove() {
                removed_pages.push((index, page.clone()));
            }

            let Some(next_index) = cursor_mut.next_present() else {
                break;
            };
            index = next_index as usize;
            if index >= page_idx_range.end {
                break;
            }
        }

        removed_pages
    }
}

impl Deref for BackedVmo<'_> {
    type Target = Vmo;

    fn deref(&self) -> &Self::Target {
        self.vmo
    }
}

pub(crate) fn get_page_idx_range(vmo_offset_range: &Range<usize>) -> Range<usize> {
    let start = vmo_offset_range.start.align_down(PAGE_SIZE);
    let end = vmo_offset_range.end.align_up(PAGE_SIZE);
    (start / PAGE_SIZE)..(end / PAGE_SIZE)
}

#[derive(Debug, Default)]
pub(crate) struct WritableMappingStatus(AtomicIsize);

impl WritableMappingStatus {
    pub(crate) fn map(&self) -> Result<()> {
        self.0
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                (v >= 0).then(|| v + 1)
            })
            .map_err(|_| errno!(EPERM, "writable mappings have been denied"))?;
        Ok(())
    }

    pub(crate) fn deny(&self) -> Result<()> {
        self.0
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                (v <= 0).then_some(-1)
            })
            .map_err(|_| errno!(EBUSY, "there are still active writable mappings"))?;
        Ok(())
    }

    pub(crate) fn increment(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn decrement(&self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}
