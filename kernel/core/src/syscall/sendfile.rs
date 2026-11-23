use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{
    fs::{
        self,
        file::{
            InodeHandle, InodeType, SeekFrom, StatusFlags,
            file_table::{RawFileDesc, WithFileTable},
        },
    },
    prelude::*,
};

pub(super) fn sys_sendfile(
    out_fd: RawFileDesc,
    in_fd: RawFileDesc,
    offset_ptr: Vaddr,
    count: isize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!("raw offset ptr = 0x{:x}", offset_ptr);

    let offset = if offset_ptr == 0 {
        None
    } else {
        let offset: isize = ctx.user_space().read_val(offset_ptr)?;
        if offset < 0 {
            return Err(errno!(EINVAL, "offset cannot be negative"));
        }
        Some(offset)
    };

    debug!(
        "out_fd = {}, in_fd = {}, offset = {:x?}, count = 0x{:x}",
        out_fd, in_fd, offset, count
    );

    if offset.is_some_and(|off| off.checked_add(count).is_none()) {
        return Err(errno!(EINVAL, "offset + count overflows"));
    }

    let mut count = if count < 0 {
        return Err(errno!(EINVAL, "count cannot be negative"));
    } else {
        count.cast_unsigned()
    };

    let (out_file, in_file) = ctx
        .thread_local
        .borrow_file_table_mut()
        .read_with(|inner| {
            let out_file = inner.get_file(out_fd.try_into()?)?.clone();
            let in_file = inner.get_file(in_fd.try_into()?)?.clone();
            Ok::<_, Error>((out_file, in_file))
        })?;

    const MAX_COUNT: usize = 0x7fff_f000;
    if count > MAX_COUNT {
        count = MAX_COUNT;
    }

    let outfile_is_pipe = out_file
        .downcast_ref::<InodeHandle>()
        .is_some_and(|inode_handle| inode_handle.path().inode().type_() == InodeType::NamedPipe);

    let origin_f_pos = if offset.is_none() {
        Some(in_file.seek(SeekFrom::Current(0)).map_err(|_| {
            if outfile_is_pipe {
                warn!("sendfile from a non-seekable in_file to a pipe is not yet supported");
            }
            errno!(EINVAL, "in_file is not seekable")
        })?)
    } else {
        None
    };

    if !in_file.access_mode().is_readable() {
        return Err(errno!(EBADF, "in_file is not readable"));
    }
    if !out_file.access_mode().is_writable() {
        return Err(errno!(EBADF, "out_file is not writable"));
    }

    let in_file_is_dir = in_file
        .downcast_ref::<InodeHandle>()
        .is_some_and(|inode_handle| inode_handle.path().inode().type_() == InodeType::Dir);
    if in_file_is_dir {
        return Err(errno!(EINVAL, "in_file is a directory"));
    }

    if !outfile_is_pipe && out_file.status_flags().contains(StatusFlags::O_APPEND) {
        return Err(errno!(EINVAL, "out_file is opened with O_APPEND"));
    }

    const BUFFER_SIZE: usize = PAGE_SIZE;
    let mut buffer = vec![0u8; BUFFER_SIZE].into_boxed_slice();
    let mut total_len = 0;
    let mut offset = offset.map(|offset| offset.cast_unsigned());
    let mut short_write_occurs = false;

    while total_len < count {
        let max_readlen = buffer.len().min(count - total_len);

        let read_res = if let Some(offset) = offset {
            in_file.read_bytes_at(offset, &mut buffer[..max_readlen])
        } else {
            in_file.read_bytes(&mut buffer[..max_readlen])
        };

        let read_len = match read_res {
            Ok(len) => len,
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

        let write_res = out_file.write_bytes(&buffer[..read_len]);

        match write_res {
            Ok(len) => {
                total_len += len;
                if let Some(offset) = offset.as_mut() {
                    *offset += len;
                }
                if len < read_len {
                    short_write_occurs = true;
                    break;
                }
            }
            Err(e) => {
                if total_len > 0 {
                    warn!("error occurs when trying to write file: {:?}", e);
                    short_write_occurs = true;
                    break;
                }
                if offset.is_none() {
                    in_file.seek(SeekFrom::Start(origin_f_pos.unwrap()))?;
                }
                return Err(e);
            }
        }
    }

    if let Some(offset) = offset {
        ctx.user_space().write_val(offset_ptr, &(offset as isize))?;
    } else if short_write_occurs {
        let new_f_pos = origin_f_pos.unwrap() + total_len;
        in_file.seek(SeekFrom::Start(new_f_pos))?;
    }

    fs::vfs::notify::on_access(&in_file);
    fs::vfs::notify::on_modify(&out_file);

    Ok(SyscallReturn::Return(total_len as _))
}
