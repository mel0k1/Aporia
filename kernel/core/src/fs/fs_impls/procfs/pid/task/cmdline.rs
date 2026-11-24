use super::TidDirOps;
use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps},
        vfs::inode::Inode,
    },
    prelude::*,
    thread::Thread,
};

/// Represents the inode at `/proc/[pid]/task/[tid]/cmdline` (and also `/proc/[pid]/cmdline`).
pub(super) struct CmdlineFileOps(TidDirOps);

impl CmdlineFileOps {
    pub(super) fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self(dir.clone()), parent, mkmod!(a+r))
    }
}

impl ProcFileOps for CmdlineFileOps {
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        self.0.thread()
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let Some(process) = self.0.process() else {
            return Err(errno!(ESRCH, "the process does not exist"));
        };

        let vmar_guard = process.lock_vmar();
        let Some(init_stack_reader) = vmar_guard.init_stack_reader() else {
            return Ok(0);
        };

        init_stack_reader.argv(offset, writer)
    }
}
