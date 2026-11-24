use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{
        Pid,
        pid_table::pid_table_mut,
        posix_thread::{AsPosixThread, alien_access::AlienAccessMode},
    },
    util::{MAX_IO_VECTOR_LENGTH, UserIoVec, VmReaderArray, VmWriterArray},
};

/// Copies data between the local process and a remote process.
///
/// Implements both `process_vm_readv` and `process_vm_writev`.
#[expect(clippy::too_many_arguments)]
fn do_process_vm(
    pid: Pid,
    is_read: bool,
    local_iov_addr: Vaddr,
    local_iov_cnt: usize,
    remote_iov_addr: Vaddr,
    remote_iov_cnt: usize,
    flags: u64,
    ctx: &Context,
) -> Result<usize> {
    debug!(
        "pid = {}, is_read = {}, local_iov = 0x{:x}, local_iov_cnt = {}, \
        remote_iov = 0x{:x}, remote_iov_cnt = {}, flags = 0x{:x}",
        pid, is_read, local_iov_addr, local_iov_cnt, remote_iov_addr, remote_iov_cnt, flags
    );

    if flags != 0 {
        return Err(errno!(EINVAL, "flags must be zero"));
    }
    if local_iov_cnt > MAX_IO_VECTOR_LENGTH || remote_iov_cnt > MAX_IO_VECTOR_LENGTH {
        return Err(errno!(EINVAL, "the I/O vector contains too many buffers"));
    }

    let target_process = pid_table_mut()
        .get_process(pid)
        .ok_or_else(|| errno!(ESRCH, "the target process does not exist"))?;

    // Accessing the memory of a remote process requires the same permissions
    // as attaching to it with `ptrace`.
    target_process
        .main_thread()
        .as_posix_thread()
        .unwrap()
        .check_alien_access_from(
            current_thread!().as_posix_thread().unwrap(),
            AlienAccessMode::ATTACH_WITH_FS_CREDS,
        )
        .map_err(|_| errno!(EACCES, "alien access is denied"))?;

    let user_space = ctx.user_space();

    // Both the local and the remote I/O vectors are provided in the address
    // space of the calling process; only the buffers of the remote I/O vector
    // live in the address space of the target process.
    let remote_iovs = parse_remote_iovecs(&user_space, remote_iov_addr, remote_iov_cnt)?;

    let vmar_guard = target_process.lock_vmar();
    let Some(vmar) = vmar_guard.as_ref() else {
        return Err(errno!(ESRCH, "the target process does not exist"));
    };

    const BOUNCE_LEN: usize = PAGE_SIZE;
    let mut bounce_buf = vec![0u8; BOUNCE_LEN];
    let mut total_len = 0;

    let mut remote_iov_iter = remote_iovs.into_iter();
    let mut remote_iov = None;

    if is_read {
        let mut writer_array =
            VmWriterArray::from_user_io_vecs(&user_space, local_iov_addr, local_iov_cnt)?;
        'outer: for writer in writer_array.writers_mut() {
            while writer.has_avail() {
                let (remote_base, remote_len) = match remote_iov.as_mut() {
                    Some((base, len)) if *len > 0 => (*base, *len),
                    _ => match remote_iov_iter.next() {
                        Some((base, len)) => {
                            remote_iov = Some((base, len));
                            (base, len)
                        }
                        None => break 'outer,
                    },
                };

                let chunk_len = writer.avail().min(remote_len).min(BOUNCE_LEN);

                match vmar.read_alien(
                    remote_base,
                    &mut VmWriter::from(&mut bounce_buf[..chunk_len]).to_fallible(),
                ) {
                    Ok(read_len) => {
                        let written_len = writer
                            .write_fallible(
                                &mut VmReader::from(&bounce_buf[..read_len]).to_fallible(),
                            )
                            .map_err(|(err, _)| err)?;
                        total_len += written_len;
                        let (remote_base, remote_remaining) = remote_iov.as_mut().unwrap();
                        *remote_base += written_len;
                        *remote_remaining -= written_len;
                        if written_len < read_len {
                            // The local buffer could not take all the bytes
                            // read from the remote process. This indicates an
                            // unexpected error on the local side.
                            break 'outer;
                        }
                    }
                    Err((_, 0)) => {
                        if total_len > 0 {
                            break 'outer;
                        }
                        return Err(errno!(EIO, "the remote memory is inaccessible"));
                    }
                    Err((_, read_len)) => {
                        let _ = writer.write_fallible(
                            &mut VmReader::from(&bounce_buf[..read_len]).to_fallible(),
                        );
                        total_len += read_len;
                        break 'outer;
                    }
                }
            }
        }
    } else {
        let mut reader_array =
            VmReaderArray::from_user_io_vecs(&user_space, local_iov_addr, local_iov_cnt)?;
        'outer: for reader in reader_array.readers_mut() {
            while reader.has_remain() {
                let (remote_base, remote_len) = match remote_iov.as_mut() {
                    Some((base, len)) if *len > 0 => (*base, *len),
                    _ => match remote_iov_iter.next() {
                        Some((base, len)) => {
                            remote_iov = Some((base, len));
                            (base, len)
                        }
                        None => break 'outer,
                    },
                };

                let chunk_len = reader.remain().min(remote_len).min(BOUNCE_LEN);

                let read_len = reader
                    .read_fallible(&mut VmWriter::from(&mut bounce_buf[..chunk_len]))
                    .map_err(|(err, _)| err)?;
                if read_len == 0 {
                    break 'outer;
                }

                match vmar.write_alien(
                    remote_base,
                    &mut VmReader::from(&bounce_buf[..read_len]).to_fallible(),
                ) {
                    Ok(written_len) => {
                        total_len += written_len;
                        let (remote_base, remote_remaining) = remote_iov.as_mut().unwrap();
                        *remote_base += written_len;
                        *remote_remaining -= written_len;
                        if written_len < read_len {
                            break 'outer;
                        }
                    }
                    Err((_, 0)) => {
                        if total_len > 0 {
                            break 'outer;
                        }
                        return Err(errno!(EIO, "the remote memory is inaccessible"));
                    }
                    Err((_, written_len)) => {
                        total_len += written_len;
                        break 'outer;
                    }
                }
            }
        }
    }

    Ok(total_len)
}

/// Parses the remote I/O vector, whose buffers live in the address space of
/// the target process.
fn parse_remote_iovecs(
    user_space: &CurrentUserSpace<'_>,
    remote_iov_addr: Vaddr,
    remote_iov_cnt: usize,
) -> Result<Vec<(Vaddr, usize)>> {
    let mut remote_iovs = Vec::with_capacity(remote_iov_cnt);
    for idx in 0..remote_iov_cnt {
        let addr = remote_iov_addr + idx * size_of::<UserIoVec>();
        let uiov: UserIoVec = user_space.read_val(addr)?;
        if uiov.len < 0 {
            return Err(errno!(EINVAL, "the I/O buffer length cannot be negative"));
        }
        remote_iovs.push((uiov.base, uiov.len as usize));
    }
    Ok(remote_iovs)
}

pub(super) fn sys_process_vm_readv(
    pid: u64,
    local_iov_addr: u64,
    local_iov_cnt: u64,
    remote_iov_addr: u64,
    remote_iov_cnt: u64,
    flags: u64,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let res = do_process_vm(
        pid as Pid,
        true,
        local_iov_addr as Vaddr,
        local_iov_cnt as usize,
        remote_iov_addr as Vaddr,
        remote_iov_cnt as usize,
        flags,
        ctx,
    )?;
    Ok(SyscallReturn::Return(res as _))
}

pub(super) fn sys_process_vm_writev(
    pid: u64,
    local_iov_addr: u64,
    local_iov_cnt: u64,
    remote_iov_addr: u64,
    remote_iov_cnt: u64,
    flags: u64,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let res = do_process_vm(
        pid as Pid,
        false,
        local_iov_addr as Vaddr,
        local_iov_cnt as usize,
        remote_iov_addr as Vaddr,
        remote_iov_cnt as usize,
        flags,
        ctx,
    )?;
    Ok(SyscallReturn::Return(res as _))
}
