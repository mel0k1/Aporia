use super::{
    StaticEntryWithOps,
    template::{
        ProcDir, ProcDirOps, ReaddirEntry, listed_entries_from_table, lookup_child_from_table,
        visit_listed_entries,
    },
};
use crate::{
    fs::{
        file::{InodeType, mkmod},
        procfs::pid::task::TaskDirOps,
        vfs::inode::{Inode, RevalidationPolicy},
    },
    prelude::*,
    process::pid_table::{PidEntry, PidEntryType},
    thread::Thread,
};

mod task;
pub(super) use task::TidDirOps;

pub(super) struct PidDirOps(TidDirOps);

impl PidDirOps {
    pub(super) fn new_inode(pid_entry: Arc<PidEntry>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        let this = Self(TidDirOps::new(pid_entry));
        ProcDir::new(this, parent, mkmod!(a+rx))
    }

    pub(super) fn pid_entry(&self) -> &Arc<PidEntry> {
        self.0.pid_entry()
    }

    pub(super) fn tid_dir_ops(&self) -> &TidDirOps {
        &self.0
    }

    const STATIC_ENTRIES: &[StaticEntryWithOps<PidDirOps>] = &[
        ("task", InodeType::Dir, TaskDirOps::new_inode),
        (
            "stat",
            InodeType::File,
            task::stat::StatFileOps::new_process_inode,
        ),
    ];
}

impl ProcDirOps for PidDirOps {
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        self.0.thread()
    }

    fn lookup_child(&self, this_dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        if self.0.pid_entry().type_().is_none() {
            return Err(errno!(ESRCH, "the process does not exist"));
        }

        if let Some(child) = lookup_child_from_table(name, Self::STATIC_ENTRIES, |f| {
            (f)(self, this_dir.this_weak().clone())
        }) {
            return Ok(child);
        }

        self.0
            .lookup_static_child(this_dir.this_weak().clone(), name)
    }

    fn revalidation_policy(&self) -> RevalidationPolicy {
        RevalidationPolicy::REVALIDATE_EXISTS
    }

    fn revalidate_exists(&self, _name: &str, _child: &dyn Inode) -> bool {
        matches!(self.pid_entry().type_(), Some(PidEntryType::Process))
    }

    fn visit_entries_from_offset<'a, F>(&'a self, offset: usize, visit_fn: F) -> Result<()>
    where
        F: FnMut(ReaddirEntry<'a>) -> Result<()>,
    {
        if self.0.pid_entry().type_().is_none() {
            return Err(errno!(ENOENT, "the process does not exist"));
        }

        const PROCESS_OVERRIDE_NAMES: &[&str] = &["stat"];

        let listed_entries = listed_entries_from_table(Self::STATIC_ENTRIES).chain(
            self.0
                .static_listed_entries()
                .filter(|entry| !PROCESS_OVERRIDE_NAMES.contains(&entry.name())),
        );

        visit_listed_entries(offset, listed_entries, visit_fn)
    }
}
