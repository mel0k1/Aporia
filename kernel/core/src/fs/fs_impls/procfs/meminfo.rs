use aster_util::printer::VmPrinter;

use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps},
        vfs::inode::Inode,
    },
    prelude::*,
};

pub(super) struct MemInfoFileOps;

impl MemInfoFileOps {
    pub(super) fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self, parent, mkmod!(a+r))
    }
}

impl ProcFileOps for MemInfoFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        let total = crate::vm::mem_total();
        let available = osdk_frame_allocator::load_total_free_size();

        let total = total / 1024;
        let available = available / 1024;

        writeln!(printer, "MemTotal:\t{} kB", total)?;
        writeln!(printer, "MemFree:\t{} kB", available)?;
        writeln!(printer, "MemAvailable:\t{} kB", available)?;
        writeln!(printer, "SwapTotal:\t0 kB")?;
        writeln!(printer, "SwapFree:\t0 kB")?;

        let slab = crate::vm::mem_slab() / 1024;
        writeln!(printer, "Slab:\t{} kB", slab)?;

        Ok(printer.bytes_written())
    }
}
