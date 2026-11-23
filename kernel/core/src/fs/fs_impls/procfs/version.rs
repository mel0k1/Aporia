use aster_util::printer::VmPrinter;

use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps},
        vfs::inode::Inode,
    },
    net::uts_ns::UtsName,
    prelude::*,
};

pub(super) struct VersionFileOps;

impl VersionFileOps {
    pub(super) fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self, parent, mkmod!(a+r))
    }
}

impl ProcFileOps for VersionFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        let sysname = UtsName::SYSNAME;
        let release = UtsName::RELEASE;
        let version = UtsName::VERSION;

        let compile_by = option_env!("OSDK_BUILD_USERNAME").unwrap_or("unknown");
        let compile_host = option_env!("OSDK_BUILD_HOSTNAME").unwrap_or("unknown");
        let compiler = option_env!("OSDK_BUILD_RUSTC").unwrap_or("unknown");

        writeln!(
            printer,
            "{} version {} ({}@{}) ({}) {}",
            sysname, release, compile_by, compile_host, compiler, version
        )?;
        Ok(printer.bytes_written())
    }
}
