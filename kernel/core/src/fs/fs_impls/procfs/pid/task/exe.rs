use super::TidDirOps;
use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcSym, ProcSymOps},
        vfs::inode::{Inode, SymbolicLink},
    },
    prelude::*,
    thread::Thread,
};

/// Represents the inode at `/proc/[pid]/task/[tid]/exe` (and also `/proc/[pid]/exe`).
pub(super) struct ExeSymOps(TidDirOps);

impl ExeSymOps {
    pub(super) fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcSym::new(Self(dir.clone()), parent, mkmod!(a+rwx))
    }
}

impl ProcSymOps for ExeSymOps {
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        self.0.thread()
    }

    fn read_link(&self) -> Result<SymbolicLink> {
        let Some(process) = self.0.process() else {
            return Err(errno!(ESRCH, "the process does not exist"));
        };

        let vmar_guard = process.lock_vmar();
        let Some(vmar) = vmar_guard.as_ref() else {
            return Err(errno!(ENOENT, "the process has exited"));
        };
        let path = vmar.process_vm().executable_path().clone();

        Ok(SymbolicLink::Path(path))
    }
}
