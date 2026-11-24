use aster_util::printer::VmPrinter;

use super::TidDirOps;
use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps},
        vfs::inode::Inode,
    },
    prelude::*,
    process::Uid,
    thread::Thread,
};

/// Represents the inode at `/proc/[pid]/task/[tid]/uid_map` (and also `/proc/[pid]/uid_map`).
pub(super) struct UidMapFileOps(TidDirOps);

impl UidMapFileOps {
    pub(super) fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self(dir.clone()), parent, mkmod!(a+r, u+w))
    }
}

impl ProcFileOps for UidMapFileOps {
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        self.0.thread()
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        writeln!(
            printer,
            "{:>10} {:>10} {:>10}",
            0,
            0,
            u32::from(Uid::INVALID)
        )?;

        Ok(printer.bytes_written())
    }
}
