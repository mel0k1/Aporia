use super::TidDirOps;
use crate::{
    events::IoEvents,
    fs::{
        file::{AccessMode, PerOpenFileOps, StatusFlags, mkmod},
        procfs::template::{ProcFile, ProcFileOpsByHandle},
        vfs::inode::{FileOps, Inode},
    },
    prelude::*,
    process::{
        VmarSnapshot,
        posix_thread::{AsPosixThread, alien_access::AlienAccessMode},
        signal::{PollHandle, Pollable},
    },
    thread::Thread,
};

pub(super) struct MemFileOps(TidDirOps);

impl MemFileOps {
    pub(super) fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self(dir.clone()), parent, mkmod!(u+rw))
    }
}

impl ProcFileOpsByHandle for MemFileOps {
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        self.0.thread()
    }

    fn open(
        &self,
        _access_mode: AccessMode,
        _status_flags: StatusFlags,
    ) -> Result<Box<dyn PerOpenFileOps>> {
        let Some(process) = self.0.process() else {
            return Err(errno!(ESRCH, "the process does not exist"));
        };
        let vmar_guard = process.lock_vmar();

        process
            .main_thread()
            .as_posix_thread()
            .unwrap()
            .check_alien_access_from(
                current_thread!().as_posix_thread().unwrap(),
                AlienAccessMode::ATTACH_WITH_FS_CREDS,
            )
            .map_err(|_| errno!(EACCES, "alien access is denied"))?;

        let vmar = vmar_guard.snapshot();
        Ok(Box::new(MemFileHandle(self.0.clone(), vmar)))
    }
}

struct MemFileHandle(TidDirOps, VmarSnapshot);

impl Pollable for MemFileHandle {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl FileOps for MemFileHandle {
    fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        let Some(process) = self.0.process() else {
            return Ok(0);
        };
        let vmar_guard = process.lock_vmar();
        if !vmar_guard.is_same_as(&self.1) {
            return Ok(0);
        }
        let Some(vmar) = vmar_guard.as_ref() else {
            return Ok(0);
        };

        match vmar.read_alien(offset, writer) {
            Ok(bytes) => Ok(bytes),
            Err((_, 0)) => Err(errno!(EIO)),
            Err((_, bytes)) => Ok(bytes),
        }
    }

    fn write_at(
        &self,
        offset: usize,
        reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        let Some(process) = self.0.process() else {
            return Ok(0);
        };
        let vmar_guard = process.lock_vmar();
        if !vmar_guard.is_same_as(&self.1) {
            return Ok(0);
        }
        let Some(vmar) = vmar_guard.as_ref() else {
            return Ok(0);
        };

        match vmar.write_alien(offset, reader) {
            Ok(bytes) => Ok(bytes),
            Err((_, 0)) => Err(errno!(EIO)),
            Err((_, bytes)) => Ok(bytes),
        }
    }
}

impl PerOpenFileOps for MemFileHandle {
    fn check_seekable(&self) -> Result<()> {
        Ok(())
    }

    fn is_offset_aware(&self) -> bool {
        true
    }
}
