use aster_fuse::{FsyncFlags, FuseOpenFlags, ops::init::FuseInitFlags2};
use ostd::warn;

use super::{
    inode::{VirtioFsInode, WriteOffset},
    open_handle::VirtioFsOpenHandle,
};
use crate::{
    events::IoEvents,
    fs::{
        file::{MappableObject, PerOpenFileOps, StatusFlags, SyncMode},
        vfs::inode::{FileOps, Inode},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    thread::work_queue::{self, WorkPriority},
    vm::vmar::FileMmapRequest,
};

pub(super) struct VirtioFsFile {
    inode: Arc<VirtioFsInode>,
    open_handle: Arc<VirtioFsOpenHandle>,
    cache_policy: CachePolicy,
}

impl VirtioFsFile {
    pub(super) fn new(
        inode: Arc<VirtioFsInode>,
        open_handle: Arc<VirtioFsOpenHandle>,
        cache_policy: CachePolicy,
    ) -> Self {
        Self {
            inode,
            open_handle,
            cache_policy,
        }
    }
}

impl Drop for VirtioFsFile {
    fn drop(&mut self) {
        if self.cache_policy != CachePolicy::Cached {
            return;
        }

        let fs = self.inode.fs_ref();
        let inode = self.inode.clone();
        let open_handle = self.open_handle.clone();

        work_queue::submit_work_func(
            move || {
                let _fs_guard = &fs;
                if let Err(err) = inode.invalidate_whole_page_cache() {
                    warn!(
                        "virtiofs flush before release failed for inode {:?}: {:?}",
                        inode.nodeid(),
                        err
                    );
                }

                let _ = &open_handle;
            },
            WorkPriority::Normal,
        );
    }
}

impl Pollable for VirtioFsFile {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl FileOps for VirtioFsFile {
    fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        let fh = self.open_handle.fh();
        let file_flags = self.open_handle.access_mode() as u32 | status_flags.bits();

        if self.cache_policy == CachePolicy::Cached && !status_flags.contains(StatusFlags::O_DIRECT)
        {
            self.inode.cached_read_at(offset, writer, fh, file_flags)
        } else {
            self.inode.direct_read_at(offset, writer, fh, file_flags)
        }
    }

    fn write_at(
        &self,
        offset: usize,
        reader: &mut VmReader,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        let fh = self.open_handle.fh();
        let file_flags = self.open_handle.access_mode() as u32 | status_flags.bits();

        let write_offset = if status_flags.contains(StatusFlags::O_APPEND) {
            self.inode.revalidate_attr(Some(fh))?;
            WriteOffset::Append
        } else {
            WriteOffset::Absolute(offset)
        };

        if self.cache_policy == CachePolicy::Cached
            && !status_flags.intersects(StatusFlags::O_APPEND | StatusFlags::O_DIRECT)
        {
            self.inode
                .cached_write_at(write_offset, reader, fh, file_flags)
        } else {
            self.inode
                .direct_write_at(write_offset, reader, fh, file_flags)
        }
    }
}

impl PerOpenFileOps for VirtioFsFile {
    fn mappable(&self, request: FileMmapRequest) -> Result<MappableObject<'_>> {
        let is_mmap_allowed = !self
            .open_handle
            .open_flags()
            .contains(FuseOpenFlags::FOPEN_DIRECT_IO)
            || !request.is_shared()
            || self
                .inode
                .fs_ref()
                .session()
                .negotiated_flags2()
                .contains(FuseInitFlags2::DIRECT_IO_ALLOW_MMAP);

        if !is_mmap_allowed {
            return Err(errno!(ENODEV, "the file is not mappable"));
        }

        self.inode
            .page_cache()
            .map(MappableObject::Vmo)
            .ok_or_else(|| errno!(ENODEV, "the file is not mappable"))
    }

    fn check_seekable(&self) -> Result<()> {
        if self
            .open_handle
            .open_flags()
            .intersects(FuseOpenFlags::FOPEN_STREAM | FuseOpenFlags::FOPEN_NONSEEKABLE)
        {
            return Err(errno!(ESPIPE, "the file is not seekable"));
        }
        Ok(())
    }

    fn is_offset_aware(&self) -> bool {
        true
    }

    fn seek_end(&self) -> Result<Option<usize>> {
        self.inode.revalidate_attr(Some(self.open_handle.fh()))?;

        Ok(Some(self.inode.size()))
    }

    fn sync(&self, mode: SyncMode) -> Result<()> {
        self.inode.sync(mode)?;

        let fsync_flags = match mode {
            SyncMode::Data => FsyncFlags::FDATASYNC,
            SyncMode::Full => FsyncFlags::empty(),
        };

        self.inode.fs_ref().session().fsync(
            self.inode.nodeid(),
            self.open_handle.fh(),
            fsync_flags,
        )?;

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CachePolicy {
    Cached,
    Direct,
}
