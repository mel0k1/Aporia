use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{
    events::IoEvents,
    fs::{
        file::{
            FileLike, InodeHandle, SeekFrom,
            file_table::{RawFileDesc, WithFileTable, get_file_fast},
        },
        pipe::{AnonPipeInode, Pipe},
    },
    prelude::*,
    process::signal::Poller,
    util::{MAX_IO_VECTOR_LENGTH, VmReaderArray},
};

const SPLICE_F_MOVE: u32 = 0x01;
const SPLICE_F_NONBLOCK: u32 = 0x02;
const SPLICE_F_MORE: u32 = 0x04;
const SPLICE_F_GIFT: u32 = 0x08;

const ALL_SPLICE_F: u32 = SPLICE_F_MOVE | SPLICE_F_NONBLOCK | SPLICE_F_MORE | SPLICE_F_GIFT;

/// The buffer size used to copy data between the two file descriptors.
const COPY_BUF_LEN: usize = PAGE_SIZE;

/// Returns the pipe behind the file descriptor if the file descriptor refers
/// to an anonymous pipe.
fn get_pipe(file: &Arc<dyn FileLike>) -> Option<&Pipe> {
    let inode_handle = file.downcast_ref::<InodeHandle>()?;
    let inode = inode_handle.path().inode();
    let anon_pipe_inode = inode.downcast_ref::<AnonPipeInode>()?;
    Some(anon_pipe_inode.pipe())
}

/// Reads the offset pointed to by `offset_ptr`, returning `None` if the
/// pointer is null.
///
/// If the file is not seekable but a non-null offset pointer is given, an
/// [`ESPIPE`] error is returned.
///
/// [`ESPIPE`]: Errno::ESPIPE
fn read_offset(
    file: &Arc<dyn FileLike>,
    offset_ptr: Vaddr,
    user_space: &CurrentUserSpace<'_>,
) -> Result<Option<usize>> {
    if offset_ptr == 0 {
        return Ok(None);
    }

    let offset: i64 = user_space.read_val(offset_ptr)?;
    if offset < 0 {
        return Err(errno!(EINVAL, "the offset cannot be negative"));
    }

    // A non-null offset pointer requires a seekable file.
    file.seek(SeekFrom::Current(0))
        .map_err(|_| errno!(ESPIPE, "the file is not seekable but an offset is given"))?;

    Ok(Some(offset as usize))
}

pub(super) fn sys_splice(
    fd_in: RawFileDesc,
    off_in_ptr: Vaddr,
    fd_out: RawFileDesc,
    off_out_ptr: Vaddr,
    len: usize,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "fd_in = {}, off_in = 0x{:x}, fd_out = {}, off_out = 0x{:x}, len = 0x{:x}, flags = 0x{:x}",
        fd_in, off_in_ptr, fd_out, off_out_ptr, len, flags
    );

    if flags & !ALL_SPLICE_F != 0 {
        return Err(errno!(EINVAL, "invalid splice flags"));
    }

    if len == 0 {
        return Ok(SyscallReturn::Return(0));
    }

    let user_space = ctx.user_space();
    let (in_file, out_file) = ctx
        .thread_local
        .borrow_file_table_mut()
        .read_with(|inner| {
            let in_file = inner.get_file(fd_in.try_into()?)?.clone();
            let out_file = inner.get_file(fd_out.try_into()?)?.clone();
            Ok::<_, Error>((in_file, out_file))
        })?;

    if !in_file.access_mode().is_readable() {
        return Err(errno!(EBADF, "fd_in is not readable"));
    }
    if !out_file.access_mode().is_writable() {
        return Err(errno!(EBADF, "fd_out is not writable"));
    }

    let mut in_offset = read_offset(&in_file, off_in_ptr, &user_space)?;
    let out_offset = read_offset(&out_file, off_out_ptr, &user_space)?;

    let origin_in_offset = if in_offset.is_none() {
        in_file.seek(SeekFrom::Current(0)).ok()
    } else {
        None
    };

    let mut total_len = 0;
    let mut out_offset = out_offset;
    {
        let mut buf = vec![0u8; COPY_BUF_LEN];
        while total_len < len {
            let chunk_len = (len - total_len).min(COPY_BUF_LEN);

            let read_res = match in_offset {
                Some(offset) => in_file.read_bytes_at(offset, &mut buf[..chunk_len]),
                None => in_file.read_bytes(&mut buf[..chunk_len]),
            };

            let read_len = match read_res {
                Ok(len) => len,
                Err(err) => {
                    if total_len > 0 {
                        warn!("error occurs when trying to read fd_in: {:?}", err);
                        break;
                    }
                    return Err(err);
                }
            };

            if read_len == 0 {
                break;
            }

            let write_res = match out_offset {
                Some(offset) => out_file.write_bytes_at(offset, &buf[..read_len]),
                None => out_file.write_bytes(&buf[..read_len]),
            };

            match write_res {
                Ok(written_len) => {
                    total_len += written_len;
                    if let Some(offset) = in_offset.as_mut() {
                        *offset += written_len;
                    }
                    if let Some(offset) = out_offset.as_mut() {
                        *offset += written_len;
                    }
                    if written_len < read_len {
                        break;
                    }
                }
                Err(err) => {
                    if total_len > 0 {
                        warn!("error occurs when trying to write fd_out: {:?}", err);
                        break;
                    }
                    if in_offset.is_none()
                        && let Some(origin_offset) = origin_in_offset
                    {
                        in_file.seek(SeekFrom::Start(origin_offset))?;
                    }
                    return Err(err);
                }
            }
        }
    }

    // Update the user-provided offsets to reflect the transferred bytes.
    if off_in_ptr != 0 {
        let offset = in_offset.unwrap_or(0) as i64;
        user_space.write_val(off_in_ptr, &offset)?;
    }
    if off_out_ptr != 0 {
        let offset = out_offset.unwrap_or(0) as i64;
        user_space.write_val(off_out_ptr, &offset)?;
    }

    Ok(SyscallReturn::Return(total_len as _))
}

pub(super) fn sys_vmsplice(
    fd: RawFileDesc,
    iov_ptr: Vaddr,
    iov_cnt: usize,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "fd = {}, iov = 0x{:x}, iov_cnt = {}, flags = 0x{:x}",
        fd, iov_ptr, iov_cnt, flags
    );

    if flags & !ALL_SPLICE_F != 0 {
        return Err(errno!(EINVAL, "invalid vmsplice flags"));
    }

    if iov_cnt > MAX_IO_VECTOR_LENGTH {
        return Err(errno!(EINVAL, "the I/O vector contains too many buffers"));
    }

    let user_space = ctx.user_space();
    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, fd.try_into()?);

    // `vmsplice` transfers data from user memory to a pipe, so the file
    // descriptor must refer to a pipe opened for writing.
    if get_pipe(&file).is_none() {
        return Err(errno!(EINVAL, "fd does not refer to a pipe"));
    }
    if !file.access_mode().is_writable() {
        return Err(errno!(EBADF, "fd is not writable"));
    }

    let mut reader_array = VmReaderArray::from_user_io_vecs(&user_space, iov_ptr, iov_cnt)?;

    let mut total_len = 0;
    for reader in reader_array.readers_mut() {
        match file.write(reader) {
            Ok(write_len) => total_len += write_len,
            Err(_) if total_len > 0 => break,
            Err(err) => return Err(err),
        }
        if reader.has_remain() {
            break;
        }
    }

    Ok(SyscallReturn::Return(total_len as _))
}

pub(super) fn sys_tee(
    fd_in: RawFileDesc,
    fd_out: RawFileDesc,
    len: usize,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "fd_in = {}, fd_out = {}, len = 0x{:x}, flags = 0x{:x}",
        fd_in, fd_out, len, flags
    );

    if flags & !ALL_SPLICE_F != 0 {
        return Err(errno!(EINVAL, "invalid tee flags"));
    }

    if len == 0 {
        return Ok(SyscallReturn::Return(0));
    }

    let (in_file, out_file) = ctx
        .thread_local
        .borrow_file_table_mut()
        .read_with(|inner| {
            let in_file = inner.get_file(fd_in.try_into()?)?.clone();
            let out_file = inner.get_file(fd_out.try_into()?)?.clone();
            Ok::<_, Error>((in_file, out_file))
        })?;

    // Both file descriptors must refer to pipes.
    let Some(in_pipe) = get_pipe(&in_file) else {
        return Err(errno!(EINVAL, "fd_in does not refer to a pipe"));
    };
    let Some(out_pipe) = get_pipe(&out_file) else {
        return Err(errno!(EINVAL, "fd_out does not refer to a pipe"));
    };

    if !in_file.access_mode().is_readable() {
        return Err(errno!(EBADF, "fd_in is not readable"));
    }
    if !out_file.access_mode().is_writable() {
        return Err(errno!(EBADF, "fd_out is not writable"));
    }

    if flags & SPLICE_F_NONBLOCK != 0 {
        let copied_len = in_pipe.try_tee_to(out_pipe, len)?;
        return Ok(SyscallReturn::Return(copied_len as _));
    }

    // Without `SPLICE_F_NONBLOCK`, `tee` blocks until some data becomes
    // available in the source pipe. If the source pipe already has some data
    // but the destination pipe is full, `EAGAIN` is returned instead of
    // blocking, which deviates slightly from Linux's behavior.
    if in_pipe.buffered_len() == 0 {
        let mut poller = Poller::new(None);
        loop {
            if in_pipe.buffered_len() > 0 {
                break;
            }
            if in_file
                .poll(IoEvents::IN | IoEvents::HUP, Some(poller.as_handle_mut()))
                .is_empty()
            {
                poller.wait()?;
                continue;
            }
            break;
        }
    }

    let copied_len = in_pipe.try_tee_to(out_pipe, len)?;
    Ok(SyscallReturn::Return(copied_len as _))
}
