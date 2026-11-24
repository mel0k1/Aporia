use ostd::arch::cpu::context::UserContext;

use crate::{
    prelude::*,
    process::{TermStatus, posix_thread::do_exit_group},
    syscall::SyscallReturn,
};

/// Exit all thread in a process.
pub(super) fn sys_exit_group(
    exit_code: u64,
    ctx: &Context,
    user_ctx: &mut UserContext,
) -> Result<SyscallReturn> {
    let term_status = TermStatus::Exited(exit_code as _);
    do_exit_group(term_status, ctx, user_ctx);
    Ok(SyscallReturn::Return(0))
}
