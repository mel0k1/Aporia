#![expect(unused_variables)]

use core::fmt::Display;

use super::{
    AccessMode, FileCommon, InodeHandle, SettableStatusFlags, StatusFlags, file_table::FdFlags,
    inode_handle::SeekFrom,
};
use crate::{
    fs::vfs::{inode::FallocMode, path::Path},
    net::socket::Socket,
    prelude::*,
    process::{Process, signal::Pollable},
    util::ioctl::RawIoctl,
    vm::{
        page_cache::Vmo,
        vmar::{FileMmapRequest, MapHandle},
    },
};

pub(crate) trait FileLike: Pollable + Send + Sync + Any {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        if !self.common().access_mode().is_readable() {
            return Err(errno!(EBADF, "the file is not opened for reading"));
        }
        Err(errno!(EINVAL, "read is not supported for this file type"))
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        if !self.common().access_mode().is_writable() {
            return Err(errno!(EBADF, "the file is not opened for writing"));
        }
        Err(errno!(EINVAL, "write is not supported for this file type"))
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        Err(errno!(ESPIPE, "read_at is not supported"))
    }

    fn write_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        Err(errno!(ESPIPE, "write_at is not supported"))
    }

    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32> {
        Err(errno!(ENOTTY, "ioctl is not supported"))
    }

    fn mappable(&self, request: FileMmapRequest) -> Result<MappableObject<'_>> {
        Err(errno!(ENODEV, "the file is not mappable"))
    }

    fn resize(&self, new_size: usize) -> Result<()> {
        Err(errno!(EINVAL, "resize is not supported"))
    }

    fn settable_status_flags(&self) -> SettableStatusFlags {
        SettableStatusFlags::minimal()
    }

    fn seek(&self, seek_from: SeekFrom) -> Result<usize> {
        Err(errno!(ESPIPE, "seek is not supported"))
    }

    fn fallocate(&self, _mode: FallocMode, _offset: usize, _len: usize) -> Result<()> {
        Err(errno!(
            ENODEV,
            "fallocate is not supported for this file type"
        ))
    }

    fn as_socket(&self) -> Option<&dyn Socket> {
        None
    }

    fn common(&self) -> &FileCommon;

    fn dump_proc_fdinfo(self: Arc<Self>, fd_flags: FdFlags) -> Box<dyn Display>;

    fn sync(&self, _mode: SyncMode) -> Result<()> {
        Err(errno!(EINVAL, "the file does not support synchronization"))
    }
}

impl dyn FileLike {
    pub(crate) fn path(&self) -> &Path {
        self.common().path()
    }

    pub(crate) fn access_mode(&self) -> AccessMode {
        self.common().access_mode()
    }

    pub(crate) fn status_flags(&self) -> StatusFlags {
        self.common().status_flags()
    }

    pub(crate) fn update_status_flags(&self, mut update: StatusFlagsUpdate) -> Result<()> {
        let settable_flags = self.settable_status_flags();
        if update.flags().contains(StatusFlags::O_DIRECT)
            && !settable_flags.contains(StatusFlags::O_DIRECT)
        {
            return Err(errno!(EINVAL, "the `O_DIRECT` flag is not supported"));
        }
        if !settable_flags.contains(StatusFlags::O_ASYNC) {
            update.ignore(StatusFlags::O_ASYNC);
        }

        self.common().update_status_flags(self, update);
        Ok(())
    }

    pub(crate) fn update_status_nonblock(&self, is_nonblocking: bool) {
        let update = if is_nonblocking {
            StatusFlagsUpdate::set(StatusFlags::O_NONBLOCK)
        } else {
            StatusFlagsUpdate::unset(StatusFlags::O_NONBLOCK)
        };

        self.common().update_status_flags(self, update);
    }

    pub(crate) fn update_status_async(&self, is_async: bool) -> Result<()> {
        let settable_flags = self.settable_status_flags();
        if is_async && !settable_flags.contains(StatusFlags::O_ASYNC) {
            return Err(errno!(ENOTTY, "signal-driven I/O is not supported"));
        }

        let update = if is_async {
            StatusFlagsUpdate::set(StatusFlags::O_ASYNC)
        } else {
            StatusFlagsUpdate::unset(StatusFlags::O_ASYNC)
        };

        self.common().update_status_flags(self, update);
        Ok(())
    }

    pub(crate) fn set_owner(&self, owner: Option<&Arc<Process>>) {
        self.common().owner().set(self, owner);
    }

    pub(crate) fn downcast_ref<T: FileLike>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref::<T>()
    }

    pub(crate) fn read_bytes(&self, buf: &mut [u8]) -> Result<usize> {
        let mut writer = VmWriter::from(buf).to_fallible();
        self.read(&mut writer)
    }

    pub(crate) fn write_bytes(&self, buf: &[u8]) -> Result<usize> {
        let mut reader = VmReader::from(buf).to_fallible();
        self.write(&mut reader)
    }

    pub(crate) fn read_bytes_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        let mut writer = VmWriter::from(buf).to_fallible();
        self.read_at(offset, &mut writer)
    }

    #[expect(dead_code)]
    pub(crate) fn write_bytes_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        let mut reader = VmReader::from(buf).to_fallible();
        self.write_at(offset, &mut reader)
    }

    pub(crate) fn as_socket_or_err(&self) -> Result<&dyn Socket> {
        self.as_socket()
            .ok_or_else(|| errno!(ENOTSOCK, "the file is not a socket"))
    }

    pub(crate) fn as_inode_handle_or_err(&self) -> Result<&InodeHandle> {
        self.downcast_ref()
            .ok_or_else(|| errno!(EINVAL, "the file is not related to an inode"))
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct StatusFlagsUpdate {
    mask: StatusFlags,
    flags: StatusFlags,
}

impl StatusFlagsUpdate {
    pub(crate) fn replace(mut flags: StatusFlags) -> Self {
        flags &= StatusFlags::SETFL_MASK;
        Self::new(StatusFlags::SETFL_MASK, flags)
    }

    pub(crate) fn set(mut flags: StatusFlags) -> Self {
        flags &= StatusFlags::SETFL_MASK;
        Self::new(flags, flags)
    }

    pub(crate) fn unset(mut flags: StatusFlags) -> Self {
        flags &= StatusFlags::SETFL_MASK;
        Self::new(flags, StatusFlags::empty())
    }

    fn ignore(&mut self, flags: StatusFlags) {
        self.mask.remove(flags);
        self.flags &= self.mask;
    }

    fn new(mask: StatusFlags, flags: StatusFlags) -> Self {
        Self { mask, flags }
    }

    pub(super) fn flags(self) -> StatusFlags {
        self.flags
    }

    pub(super) fn affects(self, flags: StatusFlags) -> bool {
        self.mask.intersects(flags)
    }

    pub(super) fn apply(self, current: StatusFlags) -> StatusFlags {
        (current - self.mask) | self.flags
    }
}

pub enum MappableObject<'a> {
    Vmo(Arc<Vmo>),
    Device(&'a dyn Mappable),
}

pub trait Mappable {
    fn map(&self, offset: usize, handle: MapHandle) -> Result<Box<dyn MappedObject>>;
}

pub trait MappedObject: Send + Sync + Debug {
    fn dup_at_offset(&self, offset: usize) -> Box<dyn MappedObject>;

    fn handle_page_fault(&self, _offset: usize, _handle: MapHandle) -> Result<()> {
        Err(errno!(
            EFAULT,
            "device memory page faults cannot be resolved"
        ))
    }
}

impl dyn MappedObject {
    pub(crate) fn dup(&self) -> Box<dyn MappedObject> {
        self.dup_at_offset(0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyncMode {
    Data,
    Full,
}
