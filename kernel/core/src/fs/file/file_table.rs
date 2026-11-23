use core::{
    fmt::Display,
    sync::atomic::{AtomicU8, Ordering},
};

use aster_util::{ranged_integer::RangedU32, slot_vec::SlotVec};
use ostd::sync::RwArc;

use super::file_handle::FileLike;
use crate::{
    fs::vfs::range_lock::RangeLockOwner,
    prelude::*,
    process::{ResourceType, posix_thread::FileTableRefMut},
};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct FileDesc(RangedU32<0, { i32::MAX as _ }>);

pub(crate) type RawFileDesc = i32;

impl Display for FileDesc {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0.get())
    }
}

impl From<FileDesc> for RawFileDesc {
    fn from(value: FileDesc) -> Self {
        value.0.get() as _
    }
}

impl From<FileDesc> for isize {
    fn from(value: FileDesc) -> Self {
        value.0.get() as _
    }
}

impl From<FileDesc> for u32 {
    fn from(value: FileDesc) -> Self {
        value.0.get()
    }
}

impl From<FileDesc> for u64 {
    fn from(value: FileDesc) -> Self {
        value.0.get() as _
    }
}

impl From<FileDesc> for usize {
    fn from(value: FileDesc) -> Self {
        value.0.get() as _
    }
}

impl TryFrom<RawFileDesc> for FileDesc {
    type Error = Error;

    fn try_from(value: RawFileDesc) -> Result<Self> {
        if value < 0 {
            return Err(errno!(EBADF, "negative FDs are not valid"));
        }
        Ok(Self(RangedU32::new(value.cast_unsigned())))
    }
}

pub(crate) struct FileTable {
    table: SlotVec<FileTableEntry>,
}

impl FileTable {
    pub(crate) fn new() -> RwArc<Self> {
        RwArc::new(Self {
            table: SlotVec::new(),
        })
    }

    pub(crate) fn fork_from(parent: &Self) -> RwArc<Self> {
        RwArc::new(Self {
            table: parent.table.clone(),
        })
    }

    pub(crate) fn range_lock_owner(file_table: &RwArc<Self>) -> RangeLockOwner {
        RangeLockOwner::from_address(file_table.as_ptr().addr())
    }

    fn as_range_lock_owner(&self) -> RangeLockOwner {
        RangeLockOwner::from_address(self as *const Self as usize)
    }

    pub(crate) fn len(&self) -> usize {
        self.table.slots_len()
    }

    pub(crate) fn insert(
        &mut self,
        file: Arc<dyn FileLike>,
        flags: FdFlags,
    ) -> Result<FileDesc, (Error, Arc<dyn FileLike>)> {
        self.insert_ceil(file, None, flags)
    }

    pub(crate) fn insert_ceil(
        &mut self,
        file: Arc<dyn FileLike>,
        ceil_fd: Option<FileDesc>,
        flags: FdFlags,
    ) -> Result<FileDesc, (Error, Arc<dyn FileLike>)> {
        let fd_limit = current!()
            .resource_limits()
            .get_rlimit(ResourceType::RLIMIT_NOFILE)
            .get_cur() as usize;

        let fd_range = if let Some(ceil_fd) = ceil_fd {
            let ceil_fd = usize::from(ceil_fd);
            if ceil_fd >= fd_limit {
                return Err((errno!(EINVAL, "the minimum FD reaches the FD limit"), file));
            }
            ceil_fd..fd_limit
        } else {
            0..fd_limit
        };

        let entry = FileTableEntry::new(file, flags);
        let min_free_fd = self.table.try_put_in(entry, fd_range).map_err(|entry| {
            (
                errno!(EMFILE, "no file descriptor is available in the range",),
                entry.file,
            )
        })?;

        Ok((min_free_fd as RawFileDesc).try_into().unwrap())
    }

    pub(crate) fn dup_exact(
        &mut self,
        fd: FileDesc,
        new_fd: FileDesc,
        flags: FdFlags,
    ) -> Result<Option<ClosedFile>> {
        let fd_limit = current!()
            .resource_limits()
            .get_rlimit(ResourceType::RLIMIT_NOFILE)
            .get_cur();
        if u64::from(new_fd) >= fd_limit {
            return Err(errno!(EBADF, "the new FD reaches the FD limit"));
        }

        let file = self
            .table
            .get(fd.into())
            .map(|entry| entry.file.clone())
            .ok_or_else(|| errno!(EBADF, "the FD does not exist"))?;
        let entry = FileTableEntry::new(file, flags);
        let closed_file = self.close_file(new_fd);
        self.table.put_at(new_fd.into(), entry);
        Ok(closed_file)
    }

    pub(crate) fn close_file(&mut self, fd: FileDesc) -> Option<ClosedFile> {
        let removed_entry = self.table.remove(fd.into())?;
        Some(ClosedFile::new(
            removed_entry.file,
            self.as_range_lock_owner(),
        ))
    }

    pub(crate) fn close_files_on_exec(&mut self) -> Vec<ClosedFile> {
        self.close_files(|entry| entry.flags().contains(FdFlags::CLOEXEC))
    }

    fn close_files<F>(&mut self, should_close: F) -> Vec<ClosedFile>
    where
        F: Fn(&FileTableEntry) -> bool,
    {
        let mut closed_files = Vec::new();
        let closed_fds: Vec<FileDesc> = self
            .table
            .idxes_and_items()
            .filter_map(|(idx, entry)| {
                if should_close(entry) {
                    Some((idx as RawFileDesc).try_into().unwrap())
                } else {
                    None
                }
            })
            .collect();

        for fd in closed_fds {
            closed_files.push(self.close_file(fd).unwrap());
        }

        closed_files
    }

    pub(crate) fn get_file(&self, fd: FileDesc) -> Result<&Arc<dyn FileLike>> {
        self.table
            .get(fd.into())
            .map(|entry| entry.file())
            .ok_or_else(|| errno!(EBADF, "the FD does not exist"))
    }

    pub(crate) fn get_entry(&self, fd: FileDesc) -> Result<&FileTableEntry> {
        self.table
            .get(fd.into())
            .ok_or_else(|| errno!(EBADF, "the FD does not exist"))
    }

    pub(crate) fn get_entry_mut(&mut self, fd: FileDesc) -> Result<&mut FileTableEntry> {
        self.table
            .get_mut(fd.into())
            .ok_or_else(|| errno!(EBADF, "the FD does not exist"))
    }

    pub(crate) fn fds_and_files(&self) -> impl Iterator<Item = (FileDesc, &'_ Arc<dyn FileLike>)> {
        self.table
            .idxes_and_items()
            .map(|(idx, entry)| ((idx as RawFileDesc).try_into().unwrap(), entry.file()))
    }
}

impl Drop for FileTable {
    fn drop(&mut self) {
        for (_, file) in self.fds_and_files() {
            if let Ok(inode_handle) = file.as_inode_handle_or_err() {
                inode_handle.release_range_locks(self.as_range_lock_owner());
            }
        }
    }
}

pub(crate) trait FileTableInsert {
    fn lock_and_insert(&self, file: Arc<dyn FileLike>, flags: FdFlags) -> Result<FileDesc> {
        let [fd] = self.lock_and_insert_multiple([file], flags)?;
        Ok(fd)
    }

    fn lock_and_insert_multiple<const N: usize>(
        &self,
        files: [Arc<dyn FileLike>; N],
        flags: FdFlags,
    ) -> Result<[FileDesc; N]>;
}

impl FileTableInsert for RwArc<FileTable> {
    fn lock_and_insert_multiple<const N: usize>(
        &self,
        files: [Arc<dyn FileLike>; N],
        flags: FdFlags,
    ) -> Result<[FileDesc; N]> {
        let mut fds = [None; N];
        let mut table = self.write();
        for (idx, file) in files.into_iter().enumerate() {
            match table.insert(file, flags) {
                Ok(fd) => fds[idx] = Some(fd),
                Err((err, file)) => {
                    let closed_files = fds.map(|fd| fd.and_then(|fd| table.close_file(fd)));
                    drop(table);
                    drop(closed_files);
                    drop(file);
                    return Err(err);
                }
            }
        }
        Ok(fds.map(Option::unwrap))
    }
}

#[must_use = "close cleanup runs when this value is dropped"]
pub(crate) struct ClosedFile {
    file: Arc<dyn FileLike>,
    range_lock_owner: RangeLockOwner,
}

impl ClosedFile {
    fn new(file: Arc<dyn FileLike>, range_lock_owner: RangeLockOwner) -> Self {
        Self {
            file,
            range_lock_owner,
        }
    }
}

impl Drop for ClosedFile {
    fn drop(&mut self) {
        if let Ok(inode_handle) = self.file.as_inode_handle_or_err() {
            inode_handle.release_range_locks(self.range_lock_owner);
        }
    }
}

pub(crate) trait WithFileTable {
    fn read_with<R>(&mut self, f: impl FnOnce(&FileTable) -> R) -> R;
}

impl WithFileTable for FileTableRefMut<'_> {
    fn read_with<R>(&mut self, f: impl FnOnce(&FileTable) -> R) -> R {
        let file_table = &mut **self;

        if let Some(inner) = file_table.get() {
            f(inner)
        } else {
            f(&file_table.read())
        }
    }
}

macro_rules! get_file_fast {
    ($file_table:expr, $file_desc:expr) => {{
        use alloc::borrow::Cow;

        use ostd::sync::RwArc;
        use $crate::{
            fs::file::file_table::{FileDesc, FileTable},
            process::posix_thread::FileTableRefMut,
        };

        let file_table: &mut FileTableRefMut<'_> = $file_table;
        let file_table: &mut RwArc<FileTable> = &mut **file_table;
        let file_desc: FileDesc = $file_desc;

        if let Some(inner) = file_table.get() {
            Cow::Borrowed(inner.get_file(file_desc)?)
        } else {
            Cow::Owned(file_table.read().get_file(file_desc)?.clone())
        }
    }};
}

pub(crate) use get_file_fast;

pub(crate) struct FileTableEntry {
    file: Arc<dyn FileLike>,
    flags: AtomicU8,
}

impl FileTableEntry {
    pub(crate) fn new(file: Arc<dyn FileLike>, flags: FdFlags) -> Self {
        Self {
            file,
            flags: AtomicU8::new(flags.bits()),
        }
    }

    pub(crate) fn file(&self) -> &Arc<dyn FileLike> {
        &self.file
    }

    pub(crate) fn flags(&self) -> FdFlags {
        FdFlags::from_bits(self.flags.load(Ordering::Relaxed)).unwrap()
    }

    pub(crate) fn set_flags(&self, flags: FdFlags) {
        self.flags.store(flags.bits(), Ordering::Relaxed);
    }
}

impl Clone for FileTableEntry {
    fn clone(&self) -> Self {
        Self {
            file: self.file.clone(),
            flags: AtomicU8::new(self.flags.load(Ordering::Relaxed)),
        }
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub(crate) struct FdFlags: u8 {
        const CLOEXEC = 1;
    }
}
