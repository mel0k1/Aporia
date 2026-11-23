use aster_util::printer::VmPrinter;

use super::TidDirOps;
use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps},
        vfs::inode::Inode,
    },
    prelude::*,
    process::posix_thread::{AsPosixThread, SleepingState},
    thread::Thread,
    vm::vmar::RssType,
};

pub(super) struct StatusFileOps(TidDirOps);

impl StatusFileOps {
    pub(super) fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFile::new(Self(dir.clone()), parent, mkmod!(a+r))
    }
}

impl ProcFileOps for StatusFileOps {
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        self.0.thread()
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        let Some((thread, process)) = self.0.thread_and_process() else {
            return Err(errno!(ESRCH, "the thread or the process does not exist"));
        };
        let posix_thread = thread.as_posix_thread().unwrap();
        let credentials = posix_thread.credentials();

        writeln!(
            printer,
            "Name:\t{}",
            posix_thread
                .thread_name()
                .lock()
                .as_cstr()
                .to_string_lossy()
        )?;

        let state = if thread.is_exited() {
            "Z (zombie)"
        } else {
            match posix_thread.sleeping_state() {
                SleepingState::Running => "R (running)",
                SleepingState::Interruptible => "S (sleeping)",
                SleepingState::Uninterruptible => "D (disk sleep)",
                SleepingState::StopBySignal => "T (stopped)",
                SleepingState::StopByPtrace => "t (tracing stop)",
            }
        };
        writeln!(printer, "State:\t{}", state)?;

        writeln!(printer, "Tgid:\t{}", process.pid())?;
        writeln!(printer, "Pid:\t{}", posix_thread.tid())?;
        writeln!(printer, "PPid:\t{}", process.parent().pid())?;
        writeln!(
            printer,
            "TracerPid:\t{}",
            posix_thread
                .tracer()
                .map(|tracer| tracer.as_posix_thread().unwrap().tid())
                .unwrap_or(0)
        )?;

        writeln!(
            printer,
            "Uid:\t{}\t{}\t{}\t{}",
            u32::from(credentials.ruid()),
            u32::from(credentials.euid()),
            u32::from(credentials.suid()),
            u32::from(credentials.fsuid()),
        )?;
        writeln!(
            printer,
            "Gid:\t{}\t{}\t{}\t{}",
            u32::from(credentials.rgid()),
            u32::from(credentials.egid()),
            u32::from(credentials.sgid()),
            u32::from(credentials.fsgid()),
        )?;

        writeln!(
            printer,
            "FDSize:\t{}",
            posix_thread
                .file_table()
                .lock()
                .as_ref()
                .map(|file_table| file_table.read().len())
                .unwrap_or(0)
        )?;

        if let Some(vmar_ref) = process.lock_vmar().as_ref() {
            let vsize = vmar_ref.get_mappings_total_size();
            let anon = vmar_ref.get_rss_counter(RssType::Anon) * (PAGE_SIZE / 1024);
            let file = vmar_ref.get_rss_counter(RssType::File) * (PAGE_SIZE / 1024);
            let rss = anon + file;
            writeln!(
                printer,
                "VmSize:\t{} kB\nVmRSS:\t{} kB\nRssAnon:\t{} kB\nRssFile:\t{} kB",
                vsize, rss, anon, file
            )?;
        }

        writeln!(
            printer,
            "Threads:\t{}",
            process.tasks().lock().as_slice().len()
        )?;

        writeln!(
            printer,
            "CapInh:\t{:016x}",
            credentials.inheritable_capset().bits()
        )?;
        writeln!(
            printer,
            "CapPrm:\t{:016x}",
            credentials.permitted_capset().bits()
        )?;
        writeln!(
            printer,
            "CapEff:\t{:016x}",
            credentials.effective_capset().bits()
        )?;
        writeln!(
            printer,
            "CapBnd:\t{:016x}",
            credentials.bounding_capset().bits()
        )?;
        writeln!(
            printer,
            "CapAmb:\t{:016x}",
            credentials.ambient_capset().bits()
        )?;

        Ok(printer.bytes_written())
    }
}
