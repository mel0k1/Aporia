#![short_vis_path::add(procfs)]

use self::{fs::FsDirOps, kernel::KernelDirOps};
use super::{
    StaticEntry,
    template::{ReaddirEntry, listed_entries_from_table, visit_listed_entries},
};
use crate::{
    fs::{
        file::{InodeType, mkmod},
        procfs::{
            sys::vm::VmDirOps,
            template::{ProcDir, ProcDirOps, lookup_child_from_table},
        },
        vfs::inode::Inode,
    },
    prelude::*,
};

mod fs;
mod kernel;
mod vm;

pub(in procfs) struct SysDirOps;

impl SysDirOps {
    pub(in procfs) fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcDir::new(Self, parent, mkmod!(a+rx))
    }

    const STATIC_ENTRIES: &'static [StaticEntry] = &[
        ("fs", InodeType::Dir, FsDirOps::new_inode),
        ("kernel", InodeType::Dir, KernelDirOps::new_inode),
        ("vm", InodeType::Dir, VmDirOps::new_inode),
    ];
}

impl ProcDirOps for SysDirOps {
    fn lookup_child(&self, this_dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        if let Some(child) = lookup_child_from_table(name, Self::STATIC_ENTRIES, |f| {
            (f)(this_dir.this_weak().clone())
        }) {
            return Ok(child);
        }

        Err(errno!(ENOENT, "the file does not exist"))
    }

    fn visit_entries_from_offset<'a, F>(&'a self, offset: usize, visit_fn: F) -> Result<()>
    where
        F: FnMut(ReaddirEntry<'a>) -> Result<()>,
    {
        visit_listed_entries(
            offset,
            listed_entries_from_table(Self::STATIC_ENTRIES),
            visit_fn,
        )
    }
}
