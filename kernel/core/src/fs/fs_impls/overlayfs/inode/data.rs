use super::OverlayInode;
use crate::{
    fs::{
        file::{StatusFlags, SyncMode},
        vfs::{
            inode::{FallocMode, SymbolicLink},
            path::Dentry,
        },
    },
    prelude::*,
    vm::page_cache::Vmo,
};

impl OverlayInode {
    pub(super) fn read_at_impl(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        let real = self.real_object();
        real.real_inode()
            .read_at(offset, writer, real.status_flags_for_read(status_flags))
    }

    pub(super) fn write_at_impl(
        &self,
        offset: usize,
        reader: &mut VmReader,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        if status_flags.contains(StatusFlags::O_APPEND) {
            return self.append_write(reader, status_flags);
        }
        self.writable_upper()
            .real_inode()
            .write_at(offset, reader, status_flags)
    }

    fn append_write(&self, reader: &mut VmReader, status_flags: StatusFlags) -> Result<usize> {
        let _guard = self.lock();
        let real = self.writable_upper();
        let offset = real.real_inode().size();
        real.real_inode().write_at(offset, reader, status_flags)
    }

    pub(super) fn seek_end_impl(&self) -> Option<usize> {
        self.real_object().real_inode().seek_end()
    }

    pub(super) fn resize_impl(&self, self_dentry: &Dentry, new_size: usize) -> Result<()> {
        let upper = self.writable_real_object(self_dentry)?;
        upper.real_inode().resize(upper.dentry(), new_size)
    }

    pub(super) fn fallocate_impl(&self, mode: FallocMode, offset: usize, len: usize) -> Result<()> {
        self.writable_upper()
            .real_inode()
            .fallocate(mode, offset, len)
    }

    pub(super) fn sync_impl(&self, mode: SyncMode) -> Result<()> {
        self.real_object().real_inode().sync(mode)
    }

    pub(super) fn read_link_impl(&self) -> Result<SymbolicLink> {
        self.real_object().real_inode().read_link()
    }

    pub(super) fn page_cache_impl(&self) -> Option<Arc<Vmo>> {
        self.real_object().real_inode().page_cache()
    }
}
