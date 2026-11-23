use bitflags::bitflags;

use super::SyscallReturn;
use crate::{
    fs::file::file_table::{FdFlags, RawFileDesc},
    prelude::*,
    process::ContextUnshareAdminApi,
};

bitflags! {
    #[derive(Clone, Copy, Debug)]
    struct CloseRangeFlags: u32 {
        const UNSHARE = 1 << 1;
        const CLOEXEC = 1 << 2;
    }
}

pub(super) fn sys_close(raw_fd: RawFileDesc, ctx: &Context) -> Result<SyscallReturn> {
    debug!("raw_fd = {}", raw_fd);

    let closed_file = {
        let fd = raw_fd.try_into()?;
        let file_table = ctx.thread_local.borrow_file_table();
        let mut file_table_locked = file_table.write();
        let _ = file_table_locked.get_file(fd)?;
        file_table_locked.close_file(fd).unwrap()
    };

    drop(closed_file);

    Ok(SyscallReturn::Return(0))
}

pub(super) fn sys_close_range(
    first: u32,
    last: u32,
    raw_flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!("first = {}, last = {}, flags = {}", first, last, raw_flags);

    if last < first {
        return Err(errno!(EINVAL, "the range to close is invalid"));
    }

    let flags = CloseRangeFlags::from_bits(raw_flags)
        .ok_or_else(|| errno!(EINVAL, "invalid close_range flags"))?;

    if flags.contains(CloseRangeFlags::UNSHARE) {
        ctx.unshare_files();
    }

    let file_table = ctx.thread_local.borrow_file_table();

    let mut files_to_drop = Vec::new();

    {
        let mut file_table_locked = file_table.write();

        let table_len = file_table_locked.len() as u32;
        if first >= table_len {
            return Ok(SyscallReturn::Return(0));
        }
        let actual_last = last.min(table_len - 1);

        for fd in first..=actual_last {
            let fd = fd.cast_signed().try_into().unwrap();

            if flags.contains(CloseRangeFlags::CLOEXEC) {
                if let Ok(entry) = file_table_locked.get_entry_mut(fd) {
                    entry.set_flags(entry.flags() | FdFlags::CLOEXEC);
                }
            } else if let Some(file) = file_table_locked.close_file(fd) {
                files_to_drop.push(file);
            }
        }
    }

    drop(files_to_drop);

    Ok(SyscallReturn::Return(0))
}
