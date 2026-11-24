//! This module offers `/proc/cmdline` file support, which provides
//! information about arguments passed to the kernel at boot time.
//!
//! Reference: <https://man7.org/linux/man-pages/man5/proc_cmdline.5.html>

use aster_util::printer::VmPrinter;
use ostd::boot::boot_info;

use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps},
        vfs::inode::Inode,
    },
    prelude::*,
};

/// Represents the inode at `/proc/cmdline`.
pub(super) struct CmdLineFileOps;

impl CmdLineFileOps {
    pub(super) fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self, parent, mkmod!(a+r))
    }
}

impl ProcFileOps for CmdLineFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        writeln!(printer, "{}", boot_info().kernel_cmdline)?;

        Ok(printer.bytes_written())
    }
}
