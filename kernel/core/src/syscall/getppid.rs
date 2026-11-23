use super::SyscallReturn;
use crate::prelude::*;

pub(super) fn sys_getppid(ctx: &Context) -> Result<SyscallReturn> {
    Ok(SyscallReturn::Return(ctx.process.parent().pid() as _))
}
