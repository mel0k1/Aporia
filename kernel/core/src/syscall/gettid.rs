use super::SyscallReturn;
use crate::prelude::*;

pub(super) fn sys_gettid(ctx: &Context) -> Result<SyscallReturn> {
    let tid = ctx.posix_thread.tid();
    Ok(SyscallReturn::Return(tid as _))
}
