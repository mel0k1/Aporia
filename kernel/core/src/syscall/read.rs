use super::SyscallReturn;
use crate::{
    fs,
    fs::file::file_table::{RawFileDesc, get_file_fast},
    prelude::*,
};

pub(super) fn sys_read(
    raw_fd: RawFileDesc,
    user_buf_addr: Vaddr,
    buf_len: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "raw_fd = {}, user_buf_ptr = 0x{:x}, buf_len = 0x{:x}",
        raw_fd, user_buf_addr, buf_len
    );

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, raw_fd.try_into()?);

    let read_len = {
        if buf_len != 0 {
            let user_space = ctx.user_space();
            let mut writer = user_space.writer(user_buf_addr, buf_len)?;
            file.read(&mut writer)
        } else {
            file.read_bytes(&mut [])
        }
    }
    .map_err(|err| match err.error() {
        Errno::EINTR => errno!(ERESTARTSYS),
        _ => err,
    })?;

    if read_len > 0 {
        fs::vfs::notify::on_access(&file);
    }
    Ok(SyscallReturn::Return(read_len as _))
}
