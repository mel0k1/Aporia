use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;
use bitflags::bitflags;

use crate::{
    fs::{
        file::{AccessMode, FileCommon, FileLike, InodeType, StatusFlags},
        vfs::path::Path,
    },
    prelude::*,
};

pub(crate) mod inotify;

use crate::fs::vfs::{inode::Inode, inode_ext::InodeExt};

pub(crate) struct FsEventPublisher {
    subscribers: RwLock<Vec<Arc<dyn FsEventSubscriber>>>,
    all_interesting_events: AtomicFsEvents,
    accepts_new_subscribers: AtomicBool,
}

impl Debug for FsEventPublisher {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FsEventPublisher")
            .field("num_subscribers", &self.subscribers.read().len())
            .finish_non_exhaustive()
    }
}

impl Default for FsEventPublisher {
    fn default() -> Self {
        Self::new()
    }
}

impl FsEventPublisher {
    pub(crate) fn new() -> Self {
        Self {
            subscribers: RwLock::new(Vec::new()),
            all_interesting_events: AtomicFsEvents::new(FsEvents::empty()),
            accepts_new_subscribers: AtomicBool::new(true),
        }
    }

    pub(crate) fn add_subscriber(&self, subscriber: Arc<dyn FsEventSubscriber>) -> bool {
        let mut subscribers = self.subscribers.write();

        if !self.accepts_new_subscribers.load(Ordering::Relaxed) {
            return false;
        }

        let current = self.all_interesting_events.load(Ordering::Relaxed);
        self.all_interesting_events
            .store(current | subscriber.interesting_events(), Ordering::Relaxed);

        subscribers.push(subscriber);

        true
    }

    pub(crate) fn remove_subscriber(&self, subscriber: &Arc<dyn FsEventSubscriber>) -> bool {
        let mut subscribers = self.subscribers.write();

        let orig_len = subscribers.len();
        self.retain_and_recalc_events(&mut subscribers, |m| !Arc::ptr_eq(m, subscriber));

        let removed = subscribers.len() != orig_len;
        if removed {
            subscriber.deliver_event(FsEvents::IN_IGNORED, None);
        }

        removed
    }

    pub(crate) fn remove_all_subscribers(&self) -> usize {
        let mut subscribers = self.subscribers.write();

        for subscriber in subscribers.iter() {
            subscriber.deliver_event(FsEvents::IN_IGNORED, None);
        }

        let num_subscribers = subscribers.len();
        subscribers.clear();

        self.all_interesting_events
            .store(FsEvents::empty(), Ordering::Relaxed);

        num_subscribers
    }

    pub(crate) fn disable_new_and_remove_subscribers(&self) -> usize {
        self.accepts_new_subscribers.store(false, Ordering::Relaxed);

        self.remove_all_subscribers()
    }

    pub(crate) fn publish_event(&self, events: FsEvents, name: Option<String>) {
        let interesting = self.all_interesting_events.load(Ordering::Relaxed);
        if !interesting.intersects(events) {
            return;
        }

        let subscribers = self.subscribers.read();
        let mut has_oneshot = false;
        for subscriber in subscribers.iter() {
            has_oneshot |= subscriber.deliver_event(events, name.clone());
        }
        drop(subscribers);

        if has_oneshot {
            let mut subscribers = self.subscribers.write();
            self.retain_and_recalc_events(&mut subscribers, |m| !m.is_oneshot_and_dead());
        }
    }

    pub(crate) fn update_subscriber_events(&self) {
        let mut subscribers = self.subscribers.write();
        self.retain_and_recalc_events(&mut subscribers, |_| true);
    }

    fn retain_and_recalc_events<F>(
        &self,
        subscribers: &mut Vec<Arc<dyn FsEventSubscriber>>,
        mut pred: F,
    ) where
        F: FnMut(&Arc<dyn FsEventSubscriber>) -> bool,
    {
        let mut new_events = FsEvents::empty();
        subscribers.retain(|subscriber| {
            if pred(subscriber) {
                new_events |= subscriber.interesting_events();
                true
            } else {
                false
            }
        });
        self.all_interesting_events
            .store(new_events, Ordering::Relaxed);
    }

    #[expect(dead_code)]
    pub(crate) fn find_subscriber_and_process<F, T>(&self, mut matcher: F) -> Option<T>
    where
        F: FnMut(&Arc<dyn FsEventSubscriber>) -> Option<T>,
    {
        let subscribers = self.subscribers.read();
        for subscriber in subscribers.iter() {
            if let Some(result) = matcher(subscriber) {
                return Some(result);
            }
        }
        None
    }
}

pub(crate) trait FsEventSubscriber: Any + Send + Sync {
    fn deliver_event(&self, events: FsEvents, name: Option<String>) -> bool;

    fn interesting_events(&self) -> FsEvents;

    fn is_oneshot_and_dead(&self) -> bool {
        false
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub(crate) struct FsEvents: u32 {
        const ACCESS         = 0x00000001;
        const MODIFY         = 0x00000002;
        const ATTRIB         = 0x00000004;
        const CLOSE_WRITE    = 0x00000008;
        const CLOSE_NOWRITE  = 0x00000010;
        const OPEN           = 0x00000020;
        const MOVED_FROM     = 0x00000040;
        const MOVED_TO       = 0x00000080;
        const CREATE         = 0x00000100;
        const DELETE         = 0x00000200;
        const DELETE_SELF    = 0x00000400;
        const MOVE_SELF      = 0x00000800;
        const OPEN_EXEC      = 0x00001000;
        const UNMOUNT        = 0x00002000;
        const Q_OVERFLOW     = 0x00004000;
        const ERROR          = 0x00008000;
        const IN_IGNORED     = 0x00008000;
        const OPEN_PERM      = 0x00010000;
        const ACCESS_PERM    = 0x00020000;
        const OPEN_EXEC_PERM = 0x00040000;
        const EVENT_ON_CHILD = 0x08000000;
        const RENAME         = 0x10000000;
        const DN_MULTISHOT   = 0x20000000;
        const ISDIR          = 0x40000000;
    }
}

impl From<u32> for FsEvents {
    fn from(value: u32) -> Self {
        Self::from_bits_truncate(value)
    }
}

impl From<FsEvents> for u32 {
    fn from(value: FsEvents) -> Self {
        value.bits()
    }
}

define_atomic_version_of_integer_like_type!(FsEvents, {
    #[derive(Debug)]
    pub(super) struct AtomicFsEvents(AtomicU32);
});

pub(crate) fn on_access(file: &Arc<dyn FileLike>) {
    let path = file.path();

    if !path.fs().fs_event_subscriber_stats().has_any_subscribers() {
        return;
    }
    notify_parent(path, FsEvents::ACCESS);
}

pub(crate) fn on_modify(file: &Arc<dyn FileLike>) {
    let path = file.path();

    if !path.fs().fs_event_subscriber_stats().has_any_subscribers() {
        return;
    }
    notify_parent(path, FsEvents::MODIFY);
}

pub(crate) fn on_change(path: &Path) {
    if !path.fs().fs_event_subscriber_stats().has_any_subscribers() {
        return;
    }
    notify_parent(path, FsEvents::MODIFY);
}

pub(crate) fn on_delete(
    dir_inode: &Arc<dyn Inode>,
    inode: &Arc<dyn Inode>,
    name: impl FnOnce() -> String,
) {
    if !dir_inode
        .fs()
        .fs_event_subscriber_stats()
        .has_any_subscribers()
    {
        return;
    }
    if inode.type_() == InodeType::Dir {
        notify_inode_with_name(dir_inode, FsEvents::DELETE | FsEvents::ISDIR, name)
    } else {
        notify_inode_with_name(dir_inode, FsEvents::DELETE, name)
    }
}

pub(crate) fn on_link_count(inode: &Arc<dyn Inode>) {
    if !inode.fs().fs_event_subscriber_stats().has_any_subscribers() {
        return;
    }
    notify_inode(inode, FsEvents::ATTRIB);
}

pub(crate) fn on_inode_removed(inode: &Arc<dyn Inode>) {
    if !inode.fs().fs_event_subscriber_stats().has_any_subscribers() {
        return;
    }
    notify_inode(inode, FsEvents::DELETE_SELF);
}

pub(crate) fn on_link(
    dir_inode: &Arc<dyn Inode>,
    inode: &Arc<dyn Inode>,
    name: impl FnOnce() -> String,
) {
    if !dir_inode
        .fs()
        .fs_event_subscriber_stats()
        .has_any_subscribers()
    {
        return;
    }
    notify_inode(inode, FsEvents::ATTRIB);
    notify_inode_with_name(dir_inode, FsEvents::CREATE, name);
}

pub(crate) fn on_mkdir(dir_path: &Path, name: impl FnOnce() -> String) {
    if !dir_path
        .fs()
        .fs_event_subscriber_stats()
        .has_any_subscribers()
    {
        return;
    }
    notify_inode_with_name(dir_path.inode(), FsEvents::CREATE | FsEvents::ISDIR, name);
}

pub(crate) fn on_create(file_path: &Path, name: impl FnOnce() -> String) {
    if !file_path
        .fs()
        .fs_event_subscriber_stats()
        .has_any_subscribers()
    {
        return;
    }
    notify_inode_with_name(file_path.inode(), FsEvents::CREATE, name);
}

pub(crate) fn on_open(common: &FileCommon) {
    let Some(path) = notifiable_path(common) else {
        return;
    };
    notify_parent(path, FsEvents::OPEN);
}

pub(crate) fn on_close(common: &FileCommon) {
    let Some(path) = notifiable_path(common) else {
        return;
    };
    let events = match common.access_mode() {
        AccessMode::O_RDONLY => FsEvents::CLOSE_NOWRITE,
        _ => FsEvents::CLOSE_WRITE,
    };
    notify_parent(path, events);
}

fn notifiable_path(common: &FileCommon) -> Option<&Path> {
    if common.status_flags().contains(StatusFlags::O_PATH) {
        return None;
    }
    let path = common.path();
    if !path.fs().fs_event_subscriber_stats().has_any_subscribers() {
        return None;
    }
    Some(path)
}

pub(crate) fn on_attr_change(path: &Path) {
    if !path.fs().fs_event_subscriber_stats().has_any_subscribers() {
        return;
    }
    notify_parent(path, FsEvents::ATTRIB);
}

fn notify_parent(path: &Path, mut events: FsEvents) {
    if path.inode().type_() == InodeType::Dir {
        events |= FsEvents::ISDIR;
    }

    let parent = path.parent_within_mount();
    if let Some(parent) = parent {
        notify_inode_with_name(parent.inode(), events, || path.name());
    }
    notify_inode(path.inode(), events);
}

fn notify_inode(inode: &Arc<dyn Inode>, events: FsEvents) {
    if let Some(publisher) = inode.fs_event_publisher() {
        publisher.publish_event(events, None);
    }
}

fn notify_inode_with_name(inode: &Arc<dyn Inode>, events: FsEvents, name: impl FnOnce() -> String) {
    if let Some(publisher) = inode.fs_event_publisher() {
        publisher.publish_event(events, Some(name()));
    }
}
