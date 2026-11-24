use alloc::format;

use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcSym, ProcSymOps},
        vfs::inode::{Inode, SymbolicLink},
    },
    prelude::*,
    process::posix_thread::AsPosixThread,
};

/// Represents the inode at `/proc/self-thread`.
pub(super) struct ThreadSelfSymOps;

impl ThreadSelfSymOps {
    pub(super) fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcSym::new(Self, parent, mkmod!(a+rwx))
    }
}

impl ProcSymOps for ThreadSelfSymOps {
    fn read_link(&self) -> Result<SymbolicLink> {
        let pid = current!().pid();
        let tid = current_thread!().as_posix_thread().unwrap().tid();
        Ok(SymbolicLink::Plain(format!("{}/task/{}", pid, tid)))
    }
}
