use alloc::boxed::ThinBox;

use crate::fs::{
    file::flock::FlockList,
    vfs::{inode::Inode, notify::FsEventPublisher, range_lock::RangeLockList},
};

pub(crate) struct FsLockContext {
    range_lock_list: RangeLockList,
    flock_list: FlockList,
}

impl FsLockContext {
    pub(self) fn new() -> Self {
        Self {
            range_lock_list: RangeLockList::new(),
            flock_list: FlockList::new(),
        }
    }

    pub(crate) fn range_lock_list(&self) -> &RangeLockList {
        &self.range_lock_list
    }

    pub(crate) fn flock_list(&self) -> &FlockList {
        &self.flock_list
    }
}

pub(crate) trait InodeExt {
    fn fs_event_publisher_or_init(&self) -> &FsEventPublisher;

    fn fs_event_publisher(&self) -> Option<&FsEventPublisher>;

    fn fs_lock_context_or_init(&self) -> &FsLockContext;

    fn fs_lock_context(&self) -> Option<&FsLockContext>;
}

impl InodeExt for dyn Inode {
    fn fs_event_publisher_or_init(&self) -> &FsEventPublisher {
        self.extension()
            .group1()
            .call_once(|| ThinBox::new_unsize(FsEventPublisher::new()))
            .downcast_ref()
            .unwrap()
    }

    fn fs_event_publisher(&self) -> Option<&FsEventPublisher> {
        Some(self.extension().group1().get()?.downcast_ref().unwrap())
    }

    fn fs_lock_context_or_init(&self) -> &FsLockContext {
        self.extension()
            .group2()
            .call_once(|| ThinBox::new_unsize(FsLockContext::new()))
            .downcast_ref()
            .unwrap()
    }

    fn fs_lock_context(&self) -> Option<&FsLockContext> {
        Some(self.extension().group2().get()?.downcast_ref().unwrap())
    }
}
