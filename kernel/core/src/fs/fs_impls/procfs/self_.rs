use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcSym, ProcSymOps},
        vfs::inode::{Inode, SymbolicLink},
    },
    prelude::*,
};

pub(super) struct SelfSymOps;

impl SelfSymOps {
    pub(super) fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcSym::new(Self, parent, mkmod!(a+rwx))
    }
}

impl ProcSymOps for SelfSymOps {
    fn read_link(&self) -> Result<SymbolicLink> {
        Ok(SymbolicLink::Plain(current!().pid().to_string()))
    }
}
