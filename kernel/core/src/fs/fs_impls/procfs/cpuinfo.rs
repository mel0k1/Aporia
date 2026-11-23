use aster_util::printer::VmPrinter;
use ostd::{
    cpu::{PinCurrentCpu, all_cpus},
    cpu_local,
    task::disable_preempt,
};
use spin::Once;

use crate::{
    arch::cpu::CpuInformation,
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps},
        vfs::inode::Inode,
    },
    prelude::*,
};

pub(super) struct CpuInfoFileOps;

impl CpuInfoFileOps {
    pub(super) fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self, parent, mkmod!(a+r))
    }
}

impl ProcFileOps for CpuInfoFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        for cpu in all_cpus() {
            let cpu_info = CPU_INFORMATION.get_on_cpu(cpu).get().unwrap().to_string();
            writeln!(printer, "{}", cpu_info)?;
        }

        Ok(printer.bytes_written())
    }
}

cpu_local! {
    static CPU_INFORMATION: Once<CpuInformation> = Once::new();
}

pub(super) fn init_on_each_cpu() {
    let guard = disable_preempt();
    CPU_INFORMATION
        .get_on_cpu(guard.current_cpu())
        .call_once(|| CpuInformation::new(&guard));
}
