use core::sync::atomic::{AtomicUsize, Ordering};

use inherit_methods_macro::inherit_methods;

use crate::{
    device::tty::{
        Tty,
        vt::{
            VtDriver,
            manager::{VT_MANAGER, VtManagerGuard},
        },
    },
    events::IoEvents,
    fs::{
        file::{PerOpenFileOps, SettableStatusFlags, StatusFlags},
        vfs::{inode::FileOps, path::Path},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    util::ioctl::RawIoctl,
};

pub(super) struct VtOpenFileCounter(AtomicUsize);

impl VtOpenFileCounter {
    pub(super) const fn new() -> Self {
        Self(AtomicUsize::new(0))
    }

    fn acquire_one(&self, _guard: &VtManagerGuard<'_>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    fn release_one(&self) {
        let old_count = self.0.fetch_sub(1, Ordering::Relaxed);
        debug_assert!(old_count > 0, "releasing more files than acquired");
    }

    pub(super) fn has_open_files(&self, _guard: &VtManagerGuard<'_>) -> bool {
        self.0.load(Ordering::Relaxed) > 0
    }
}

pub(super) struct VtFile(Arc<Tty<VtDriver>>);

impl VtFile {
    pub(super) fn new(tty: Arc<Tty<VtDriver>>) -> Result<VtFile> {
        let driver = tty.driver();
        let vtm = VT_MANAGER.get().expect("`VT_MANAGER` is not initialized");
        let guard = vtm.lock();
        guard.allocate_vt(driver.index());
        driver.open_file_counter().acquire_one(&guard);

        Ok(VtFile(tty))
    }
}

impl Drop for VtFile {
    fn drop(&mut self) {
        self.0.driver().open_file_counter().release_one();
    }
}

#[inherit_methods(from = "self.0")]
impl Pollable for VtFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents;
}

impl FileOps for VtFile {
    fn read_at(
        &self,
        _offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        self.0.read(writer, status_flags)
    }

    fn write_at(
        &self,
        _offset: usize,
        reader: &mut VmReader,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        self.0.write(reader, status_flags)
    }
}

impl PerOpenFileOps for VtFile {
    fn check_seekable(&self) -> Result<()> {
        Err(errno!(ESPIPE, "the inode is a TTY"))
    }

    fn is_offset_aware(&self) -> bool {
        false
    }

    fn ioctl(&self, _path: &Path, raw_ioctl: RawIoctl) -> Result<i32> {
        self.0.ioctl(raw_ioctl)
    }

    fn settable_status_flags(&self) -> SettableStatusFlags {
        SettableStatusFlags::minimal().with_o_async()
    }
}
