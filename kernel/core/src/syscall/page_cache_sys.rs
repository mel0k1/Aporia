use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{
    fs::file::{
        InodeHandle,
        file_table::{RawFileDesc, get_file_fast},
    },
    prelude::*,
    vm::page_cache::Vmo,
};

/// The byte length of a single readahead chunk.
///
/// Readahead may be requested for an arbitrarily large range, so the data is
/// pulled into the page cache chunk by chunk to bound the scratch buffer size.
const READAHEAD_CHUNK_LEN: usize = 64 * PAGE_SIZE;

const SYNC_FILE_RANGE_WAIT_BEFORE: u32 = 1;
const SYNC_FILE_RANGE_WRITE: u32 = 2;
const SYNC_FILE_RANGE_WAIT_AFTER: u32 = 4;

const ALL_SYNC_FILE_RANGE_FLAGS: u32 =
    SYNC_FILE_RANGE_WAIT_BEFORE | SYNC_FILE_RANGE_WRITE | SYNC_FILE_RANGE_WAIT_AFTER;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct CstatRange {
    off: u64,
    len: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct Cstat {
    nr_cache: u64,
    nr_dirty: u64,
    nr_writeback: u64,
    nr_evicted: u64,
    nr_recently_evicted: u64,
}

/// Returns the page-cache VMO behind the file descriptor, if it has one.
///
/// Files without a page cache (e.g., pipes or sockets) are rejected.
fn get_page_cache_vmo(fd: RawFileDesc, ctx: &Context) -> Result<Arc<Vmo>> {
    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, fd.try_into()?);
    let inode_handle = file
        .downcast_ref::<InodeHandle>()
        .ok_or_else(|| errno!(EINVAL, "the file does not have a page cache"))?;
    let inode = inode_handle.path().inode();
    inode
        .page_cache()
        .ok_or_else(|| errno!(EINVAL, "the file does not have a page cache"))
}

pub(super) fn sys_readahead(
    fd: RawFileDesc,
    offset: i64,
    count: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!("fd = {}, offset = {}, count = 0x{:x}", fd, offset, count);

    if offset < 0 {
        return Err(errno!(EINVAL, "the offset cannot be negative"));
    }

    let vmo = {
        let mut file_table = ctx.thread_local.borrow_file_table_mut();
        let file = get_file_fast!(&mut file_table, fd.try_into()?);
        if !file.access_mode().is_readable() {
            return Err(errno!(EBADF, "the file is not open for reading"));
        }
        let inode_handle = file
            .downcast_ref::<InodeHandle>()
            .ok_or_else(|| errno!(EINVAL, "the file does not have a page cache"))?;
        let inode = inode_handle.path().inode();
        inode
            .page_cache()
            .ok_or_else(|| errno!(EINVAL, "the file does not have a page cache"))?
    };

    // Pull the pages into the page cache by reading through the cache; the
    // contents copied into the scratch buffer are discarded.
    let mut offset = offset as usize;
    let end = offset.saturating_add(count);
    while offset < end {
        let chunk_len = (end - offset).min(READAHEAD_CHUNK_LEN);
        let mut scratch = vec![0u8; chunk_len];
        let mut writer = VmWriter::from(scratch.as_mut_slice()).to_fallible();
        vmo.read(offset, &mut writer)?;

        let written = chunk_len - writer.avail();
        if written == 0 {
            break;
        }
        offset += written;
    }

    Ok(SyscallReturn::Return(0))
}

pub(super) fn sys_sync_file_range(
    fd: RawFileDesc,
    offset: i64,
    nbytes: i64,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "fd = {}, offset = {}, nbytes = 0x{:x}, flags = 0x{:x}",
        fd, offset, nbytes, flags
    );

    if flags & !ALL_SYNC_FILE_RANGE_FLAGS != 0 || flags == 0 {
        return Err(errno!(EINVAL, "invalid sync_file_range flags"));
    }

    if offset < 0 || nbytes < 0 {
        return Err(errno!(EINVAL, "offset and nbytes cannot be negative"));
    }

    let vmo = get_page_cache_vmo(fd, ctx)?;

    let offset = offset as usize;
    let end = offset.saturating_add(nbytes as usize);
    vmo.flush_dirty_range(&(offset..end))?;

    Ok(SyscallReturn::Return(0))
}

pub(super) fn sys_cachestat(
    fd: RawFileDesc,
    range_ptr: Vaddr,
    cstat_ptr: Vaddr,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "fd = {}, range = 0x{:x}, cstat = 0x{:x}, flags = 0x{:x}",
        fd, range_ptr, cstat_ptr, flags
    );

    if flags != 0 {
        return Err(errno!(EINVAL, "flags must be zero"));
    }

    let user_space = ctx.user_space();
    let cstat_range: CstatRange = user_space.read_val(range_ptr)?;
    if cstat_range.off.checked_add(cstat_range.len).is_none() {
        return Err(errno!(EINVAL, "the range overflows"));
    }

    let vmo = get_page_cache_vmo(fd, ctx)?;
    let vmo_size = vmo.size() as u64;

    // A zero length means the range extends to the end of the file.
    let off = cstat_range.off;
    let end = if cstat_range.len == 0 {
        vmo_size
    } else {
        (off + cstat_range.len).min(vmo_size)
    };

    let (nr_cache, nr_dirty) = if off < end {
        let (cached, dirty) = vmo.cached_page_count(&(off as usize..end as usize));
        (cached as u64, dirty as u64)
    } else {
        (0, 0)
    };

    user_space.write_val(
        cstat_ptr,
        &Cstat {
            nr_cache,
            nr_dirty,
            // Writeback and eviction states are not tracked; report zeros.
            nr_writeback: 0,
            nr_evicted: 0,
            nr_recently_evicted: 0,
        },
    )?;

    Ok(SyscallReturn::Return(0))
}
