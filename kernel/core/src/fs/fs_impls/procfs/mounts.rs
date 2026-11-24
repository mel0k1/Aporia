use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcSym, ProcSymOps},
        vfs::inode::{Inode, SymbolicLink},
    },
    prelude::*,
};

/// Represents the inode at `/proc/mounts`.
pub(super) struct MountsSymOps;

impl MountsSymOps {
    pub(super) fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcSym::new(Self, parent, mkmod!(a+rwx))
    }
}

impl ProcSymOps for MountsSymOps {
    fn read_link(&self) -> Result<SymbolicLink> {
        Ok(SymbolicLink::Plain("self/mounts".to_string()))
    }
}
