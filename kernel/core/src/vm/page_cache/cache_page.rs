use core::{
    borrow::Borrow,
    ops::Deref,
    sync::atomic::{AtomicBool, AtomicU8, Ordering},
};

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;
use ostd::{
    impl_untyped_frame_meta_for,
    mm::{Frame, FrameAllocOptions, HasPaddr},
    sync::WaitQueue,
};

use crate::prelude::*;

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub(crate) enum PageState {
    Uninit = 0,
    UpToDate = 1,
    Dirty = 2,
    Evicted = 3,
}

impl From<PageState> for u8 {
    fn from(value: PageState) -> Self {
        value as u8
    }
}

define_atomic_version_of_integer_like_type!(PageState, try_from = true, {
    #[derive(Debug)]
    struct AtomicPageState(AtomicU8);
});

impl PageState {
    pub(super) fn is_uninit(self) -> bool {
        matches!(self, Self::Uninit)
    }

    pub(super) fn is_up_to_date(self) -> bool {
        matches!(self, Self::UpToDate)
    }

    pub(super) fn is_dirty(self) -> bool {
        matches!(self, Self::Dirty)
    }

    pub(super) fn is_evicted(self) -> bool {
        matches!(self, Self::Evicted)
    }
}

pub(crate) type CachePage = Frame<CachePageMeta>;

#[derive(Debug)]
pub(crate) struct CachePageMeta {
    state: AtomicPageState,
    lock: AtomicBool,
    is_writing_back: AtomicBool,
}

impl Default for CachePageMeta {
    fn default() -> Self {
        Self {
            state: AtomicPageState::new(PageState::Uninit),
            lock: AtomicBool::new(false),
            is_writing_back: AtomicBool::new(false),
        }
    }
}

impl_untyped_frame_meta_for!(CachePageMeta);

pub(crate) trait CachePageExt: Sized {
    fn metadata(&self) -> &CachePageMeta;

    fn wait_queue(&self) -> &'static WaitQueue;

    #[expect(dead_code)]
    fn try_lock(self) -> Option<LockedCachePage>;

    fn try_lock_guard(&self) -> Option<LockedCachePageGuard<'_>>;

    fn lock(self) -> LockedCachePage;

    fn lock_guard(&self) -> LockedCachePageGuard<'_>;

    fn ensure_init(&self, init_fn: impl FnOnce(LockedCachePage) -> Result<()>) -> Result<()>;

    fn clear_writing_back(&self) {
        self.metadata()
            .is_writing_back
            .store(false, Ordering::Release);
        self.wait_queue().wake_all();
    }

    fn alloc_uninit() -> Result<CachePage> {
        let meta = CachePageMeta::default();
        let page = FrameAllocOptions::new()
            .zeroed(false)
            .alloc_frame_with(meta)?;
        Ok(page)
    }

    fn alloc_zero() -> Result<CachePage> {
        let meta = CachePageMeta {
            state: AtomicPageState::new(PageState::UpToDate),
            ..Default::default()
        };
        let page = FrameAllocOptions::new()
            .zeroed(true)
            .alloc_frame_with(meta)?;
        Ok(page)
    }

    fn state(&self) -> PageState {
        self.metadata().state.load(Ordering::Acquire)
    }
}

impl CachePageExt for CachePage {
    fn metadata(&self) -> &CachePageMeta {
        self.meta()
    }

    fn wait_queue(&self) -> &'static WaitQueue {
        const PAGE_SHIFT: u32 = PAGE_SIZE.trailing_zeros();
        const PAGE_WAIT_QUEUE_MASK: usize = 0xff;
        const PAGE_WAIT_QUEUE_NUM: usize = PAGE_WAIT_QUEUE_MASK + 1;

        static PAGE_WAIT_QUEUES: [WaitQueue; PAGE_WAIT_QUEUE_NUM] =
            [const { WaitQueue::new() }; PAGE_WAIT_QUEUE_NUM];

        &PAGE_WAIT_QUEUES[(self.paddr() >> PAGE_SHIFT) & PAGE_WAIT_QUEUE_MASK]
    }

    fn try_lock(self) -> Option<LockedCachePage> {
        try_lock_page(&self).map(|wait_queue| LockedCachePage::new(self, wait_queue))
    }

    fn try_lock_guard(&self) -> Option<LockedCachePageGuard<'_>> {
        try_lock_page(self).map(|wait_queue| LockedCachePage::new(self, wait_queue))
    }

    fn lock(self) -> LockedCachePage {
        let wait_queue = lock_page(&self);
        LockedCachePage::new(self, wait_queue)
    }

    fn lock_guard(&self) -> LockedCachePageGuard<'_> {
        if let Some(locked_page) = self.try_lock_guard() {
            return locked_page;
        }

        let wait_queue = lock_page(self);
        LockedCachePage::new(self, wait_queue)
    }

    fn ensure_init(&self, init_fn: impl FnOnce(LockedCachePage) -> Result<()>) -> Result<()> {
        if !self.state().is_uninit() {
            return Ok(());
        }

        let locked_page = self.lock_guard();
        if !locked_page.state().is_uninit() {
            return Ok(());
        }

        init_fn(locked_page.into_owned())
    }
}

pub(crate) struct LockedCachePage<PageRef: Borrow<CachePage> = CachePage> {
    page: Option<PageRef>,
    wait_queue: &'static WaitQueue,
}

pub(crate) type LockedCachePageGuard<'a> = LockedCachePage<&'a CachePage>;

impl<PageRef: Borrow<CachePage>> Debug for LockedCachePage<PageRef> {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("LockedCachePage")
            .field("page", &self.page.as_ref().map(|page| page.borrow()))
            .finish()
    }
}

impl<PageRef: Borrow<CachePage>> LockedCachePage<PageRef> {
    fn new(page: PageRef, wait_queue: &'static WaitQueue) -> Self {
        Self {
            page: Some(page),
            wait_queue,
        }
    }

    fn page(&self) -> &CachePage {
        self.page.as_ref().expect("page already taken").borrow()
    }

    pub(crate) fn set_up_to_date(&self) {
        self.page()
            .metadata()
            .state
            .store(PageState::UpToDate, Ordering::Release);
    }

    pub(crate) fn set_dirty(&self) {
        self.metadata()
            .state
            .store(PageState::Dirty, Ordering::Release);
    }

    pub(super) fn set_evicted(&self) {
        self.metadata()
            .state
            .store(PageState::Evicted, Ordering::Release);
    }

    pub(crate) fn set_writing_back(&self) {
        self.metadata()
            .is_writing_back
            .store(true, Ordering::Release);
    }

    pub(crate) fn wait_until_finish_writing_back(&self) {
        self.wait_queue
            .wait_until(|| (!self.is_writing_back()).then_some(()));
    }

    pub(super) fn is_writing_back(&self) -> bool {
        self.metadata().is_writing_back.load(Ordering::Acquire)
    }
}

impl LockedCachePage<CachePage> {
    pub(crate) fn unlock(mut self) -> CachePage {
        let page = self.page.take().expect("page already taken");
        unlock_page(&page, self.wait_queue);
        page
    }

    pub(super) fn detach(&mut self) -> LockedCachePage {
        let page = self.page.take().expect("page already detached");
        LockedCachePage::new(page, self.wait_queue)
    }
}

impl LockedCachePage<&CachePage> {
    pub(super) fn into_owned(mut self) -> LockedCachePage {
        let page = self.page.take().expect("page already taken").clone();
        LockedCachePage::new(page, self.wait_queue)
    }
}

impl<PageRef: Borrow<CachePage>> Deref for LockedCachePage<PageRef> {
    type Target = CachePage;

    fn deref(&self) -> &Self::Target {
        self.page()
    }
}

impl<PageRef: Borrow<CachePage>> Drop for LockedCachePage<PageRef> {
    fn drop(&mut self) {
        if let Some(page) = &self.page {
            unlock_page(page.borrow(), self.wait_queue);
        }
    }
}

fn unlock_page(page: &CachePage, wait_queue: &'static WaitQueue) {
    page.metadata().lock.store(false, Ordering::Release);
    wait_queue.wake_all();
}

fn try_lock_page(page: &CachePage) -> Option<&'static WaitQueue> {
    let wait_queue = page.wait_queue();
    page.metadata()
        .lock
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_ok()
        .then_some(wait_queue)
}

fn lock_page(page: &CachePage) -> &'static WaitQueue {
    let wait_queue = page.wait_queue();
    wait_queue.wait_until(|| {
        page.metadata()
            .lock
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()
    });
    wait_queue
}
