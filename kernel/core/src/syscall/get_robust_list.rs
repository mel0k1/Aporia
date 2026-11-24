use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{pid_table, posix_thread::RobustListHead},
};

pub(super) fn sys_get_robust_list(
    pid: i32,
    head_ptr: Vaddr,
    len_ptr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "pid = {}, head_ptr = {:#x}, len_ptr = {:#x}",
        pid, head_ptr, len_ptr
    );

    if pid != 0 && pid != ctx.posix_thread.tid().cast_signed() {
        pid_table::pid_table_mut()
            .get_thread(pid.cast_unsigned())
            .ok_or_else(|| errno!(ESRCH, "no such thread"))?;
        return Err(errno!(
            EPERM,
            "the robust list of another thread is not accessible"
        ));
    }

    let robust_list_head = ctx
        .thread_local
        .robust_list()
        .borrow()
        .unwrap_or_else(RobustListHead::new_zeroed);

    ctx.user_space().write_val(head_ptr, &robust_list_head)?;
    ctx.user_space()
        .write_val(len_ptr, &(size_of::<RobustListHead>() as usize))?;

    Ok(SyscallReturn::Return(0))
}
