use aster_util::printer::VmPrinter;

use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps},
        vfs::inode::Inode,
    },
    prelude::*,
    vm::vmar::VMAR_LOWEST_ADDR,
};

pub(super) struct MmapMinAddrFileOps;

impl MmapMinAddrFileOps {
    pub(super) fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self, parent, mkmod!(a+r, u+w))
    }
}

impl ProcFileOps for MmapMinAddrFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        writeln!(printer, "{}", VMAR_LOWEST_ADDR)?;

        Ok(printer.bytes_written())
    }

    fn write_at(&self, _offset: usize, _reader: &mut VmReader) -> Result<usize> {
        warn!("writing to `/proc/sys/vm/mmap_min_addr` is not supported");
        Err(errno!(
            EOPNOTSUPP,
            "writing to `/proc/sys/vm/mmap_min_addr` is not supported"
        ))
    }
}
