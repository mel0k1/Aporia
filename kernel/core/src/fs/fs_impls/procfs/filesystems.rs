use aster_util::printer::{VmPrinter, VmPrinterError};

use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps},
        vfs::{inode::Inode, registry::FsProperties},
    },
    prelude::*,
};

pub(super) struct FileSystemsFileOps;

impl FileSystemsFileOps {
    pub(super) fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self, parent, mkmod!(a+r))
    }
}

impl ProcFileOps for FileSystemsFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        crate::fs::vfs::registry::with_iter(|iter| -> Result<(), VmPrinterError> {
            for (fs_name, fs_type) in iter {
                if fs_type.properties().contains(FsProperties::NEED_DISK) {
                    writeln!(printer, "\t{}", fs_name)?;
                } else {
                    writeln!(printer, "nodev\t{}", fs_name)?;
                }
            }

            Ok(())
        })?;

        Ok(printer.bytes_written())
    }
}
