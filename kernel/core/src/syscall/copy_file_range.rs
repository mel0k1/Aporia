use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{
    fs::{
        self,
        file::{
            FileLike, InodeHandle, InodeType, SeekFrom, StatusFlags,
            file_table::{RawFileDesc, WithFileTable},
        },
    },
    prelude::*,
};

const MAX_COUNT: usize = 0x7fff_f000;

pub(super) fn sys_copy_file_range(
    fd_in: RawFileDesc,
    offset_in_ptr: Vaddr,
    fd_out: RawFileDesc,
    offset_out_ptr: Vaddr,
    len: usize,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "fd_in = {}, offset_in_ptr = {:#x}, fd_out = {}, offset_out_ptr = {:#x}, len = {:#x}, flags = {:#x}",
        fd_in, offset_in_ptr, fd_out, offset_out_ptr, len, flags
    );

    let (in_file, out_file) = ctx
        .thread_local
        .borrow_file_table_mut()
        .read_with(|inner| {
            let in_file = inner.get_file(fd_in.try_into()?)?.clone();
            let out_file = inner.get_file(fd_out.try_into()?)?.clone();
            Ok::<_, Error>((in_file, out_file))
        })?;

    let offset_in = read_offset(offset_in_ptr, ctx)?;
    let offset_out = read_offset(offset_out_ptr, ctx)?;

    if flags != 0 {
        return Err(errno!(EINVAL, "flags are not supported"));
    }

    let in_inode_handle = in_file
        .downcast_ref::<InodeHandle>()
        .ok_or_else(|| errno!(EINVAL, "in_file is not a regular file"))?;
    let out_inode_handle = out_file
        .downcast_ref::<InodeHandle>()
        .ok_or_else(|| errno!(EINVAL, "out_file is not a regular file"))?;

    if in_inode_handle.path().inode().type_() == InodeType::Dir
        || out_inode_handle.path().inode().type_() == InodeType::Dir
    {
        return Err(errno!(EISDIR, "copying directories is not supported"));
    }

    if !in_file.access_mode().is_readable()
        || !out_file.access_mode().is_writable()
        || out_file.status_flags().contains(StatusFlags::O_APPEND)
    {
        return Err(errno!(
            EBADF,
            "in_file is not readable or out_file is not writable"
        ));
    }

    if offset_in.is_some_and(|offset| offset.checked_add(len).is_none())
        || offset_out.is_some_and(|offset| offset.checked_add(len).is_none())
    {
        return Err(errno!(EOVERFLOW, "offset + len overflows"));
    }

    if Arc::ptr_eq(
        in_inode_handle.path().inode(),
        out_inode_handle.path().inode(),
    ) {
        let pos_in = effective_pos(in_file.as_ref(), offset_in)?;
        let pos_out = effective_pos(out_file.as_ref(), offset_out)?;

        let size_in = in_inode_handle.path().inode().metadata()?.size;
        let count = len.min(size_in.saturating_sub(pos_in));

        if pos_out + count > pos_in && pos_out < pos_in + count {
            return Err(errno!(EINVAL, "the source and destination ranges overlap"));
        }
    }

    let len = len.min(MAX_COUNT);
    let mut total_len = 0;

    const BUFFER_SIZE: usize = PAGE_SIZE;
    let mut buffer = vec![0u8; BUFFER_SIZE].into_boxed_slice();

    while total_len < len {
        let max_readlen = buffer.len().min(len - total_len);

        let read_res = if let Some(offset) = offset_in {
            in_file.read_bytes_at(offset + total_len, &mut buffer[..max_readlen])
        } else {
            in_file.read_bytes(&mut buffer[..max_readlen])
        };

        let read_len = match read_res {
            Ok(read_len) => read_len,
            Err(e) => {
                if total_len > 0 {
                    warn!("error occurs when trying to read file: {:?}", e);
                    break;
                }
                return Err(e);
            }
        };

        if read_len == 0 {
            break;
        }

        let write_res = if let Some(offset) = offset_out {
            out_file.write_bytes_at(offset + total_len, &buffer[..read_len])
        } else {
            out_file.write_bytes(&buffer[..read_len])
        };

        match write_res {
            Ok(write_len) => {
                total_len += write_len;
                if write_len < read_len {
                    break;
                }
            }
            Err(e) => {
                if total_len > 0 {
                    warn!("error occurs when trying to write file: {:?}", e);
                    break;
                }
                return Err(e);
            }
        }
    }

    if total_len > 0 {
        if let Some(offset) = offset_in {
            ctx.user_space()
                .write_val(offset_in_ptr, &(offset + total_len))?;
        }
        if let Some(offset) = offset_out {
            ctx.user_space()
                .write_val(offset_out_ptr, &(offset + total_len))?;
        }
    }

    fs::vfs::notify::on_access(&in_file);
    fs::vfs::notify::on_modify(&out_file);

    Ok(SyscallReturn::Return(total_len as _))
}

fn read_offset(offset_ptr: Vaddr, ctx: &Context) -> Result<Option<usize>> {
    if offset_ptr == 0 {
        return Ok(None);
    }

    let offset: isize = ctx.user_space().read_val(offset_ptr)?;
    if offset < 0 {
        return Err(errno!(EINVAL, "offset cannot be negative"));
    }

    Ok(Some(offset.cast_unsigned()))
}

fn effective_pos(file: &dyn FileLike, offset: Option<usize>) -> Result<usize> {
    match offset {
        Some(offset) => Ok(offset),
        None => file.seek(SeekFrom::Current(0)),
    }
}
