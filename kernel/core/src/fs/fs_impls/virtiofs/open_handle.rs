use aster_fuse::{FuseFileHandle, FuseNodeId, FuseOpenFlags, ops::release::ReleaseOptions};

use super::fs::VirtioFs;
use crate::{
    fs::file::{AccessMode, StatusFlags},
    prelude::*,
    thread::work_queue::{self, WorkPriority},
};

pub(super) struct VirtioFsOpenHandle {
    fh: FuseFileHandle,
    nodeid: FuseNodeId,
    access_mode: AccessMode,
    status_flags: StatusFlags,
    open_flags: FuseOpenFlags,
    fs: Weak<VirtioFs>,
    release_options: ReleaseOptions,
}

impl VirtioFsOpenHandle {
    pub(super) fn new(
        fh: FuseFileHandle,
        nodeid: FuseNodeId,
        access_mode: AccessMode,
        status_flags: StatusFlags,
        open_flags: FuseOpenFlags,
        fs: Weak<VirtioFs>,
        release_options: ReleaseOptions,
    ) -> Arc<Self> {
        Arc::new(Self {
            fh,
            nodeid,
            access_mode,
            status_flags,
            open_flags,
            fs,
            release_options,
        })
    }

    pub(super) fn fh(&self) -> FuseFileHandle {
        self.fh
    }

    pub(super) fn file_flags(&self) -> u32 {
        self.access_mode as u32 | self.status_flags.bits()
    }

    pub(super) fn access_mode(&self) -> AccessMode {
        self.access_mode
    }

    pub(super) fn open_flags(&self) -> FuseOpenFlags {
        self.open_flags
    }
}

impl Drop for VirtioFsOpenHandle {
    fn drop(&mut self) {
        let fs = self.fs.clone();
        let nodeid = self.nodeid;
        let fh = self.fh;
        let file_flags = self.file_flags();
        let release_options = self.release_options;

        work_queue::submit_work_func(
            move || {
                let Some(fs) = fs.upgrade() else {
                    return;
                };

                if let Err(err) = fs
                    .session()
                    .release(nodeid, fh, file_flags, release_options)
                {
                    warn!("virtiofs release failed for inode {:?}: {:?}", nodeid, err);
                }
            },
            WorkPriority::Normal,
        );
    }
}

pub(super) struct OpenHandles {
    handles: Mutex<Vec<Weak<VirtioFsOpenHandle>>>,
}

impl OpenHandles {
    pub(super) fn new() -> Self {
        Self {
            handles: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn insert(&self, handle: &Arc<VirtioFsOpenHandle>) {
        let mut handles = self.handles.lock();

        handles.retain(|h| h.strong_count() > 0);
        handles.push(Arc::downgrade(handle));
    }

    pub(super) fn find_readable_handle(&self) -> Option<Arc<VirtioFsOpenHandle>> {
        self.find_handle(AccessMode::is_readable)
    }

    pub(super) fn find_writable_handle(&self) -> Option<Arc<VirtioFsOpenHandle>> {
        self.find_handle(AccessMode::is_writable)
    }

    fn find_handle(
        &self,
        accepts_fn: impl Fn(&AccessMode) -> bool,
    ) -> Option<Arc<VirtioFsOpenHandle>> {
        let mut handles = self.handles.lock();
        let mut found = None;

        handles.retain(|handle| {
            let Some(open_handle) = handle.upgrade() else {
                return false;
            };

            if accepts_fn(&open_handle.access_mode) {
                found = Some(open_handle);
            }

            true
        });

        found
    }
}
