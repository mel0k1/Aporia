use aster_util::printer::VmPrinter;

use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps},
        vfs::inode::Inode,
    },
    prelude::*,
    process::rlimit::SYSCTL_NR_OPEN,
};

/// Represents the inode at `/proc/sys/fs/nr_open`.
pub(super) struct NrOpenFileOps;

impl NrOpenFileOps {
    pub(super) fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self, parent, mkmod!(a+r, u+w))
    }
}

impl ProcFileOps for NrOpenFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        writeln!(printer, "{}", SYSCTL_NR_OPEN)?;

        Ok(printer.bytes_written())
    }

    fn write_at(&self, _offset: usize, _reader: &mut VmReader) -> Result<usize> {
        warn!("writing to `/proc/sys/fs/nr_open` is not supported");
        Err(errno!(
            EOPNOTSUPP,
            "writing to `/proc/sys/fs/nr_open` is not supported"
        ))
    }
}
