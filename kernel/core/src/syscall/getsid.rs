use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{Pid, pid_table},
};

pub(super) fn sys_getsid(pid: Pid, ctx: &Context) -> Result<SyscallReturn> {
    debug!("pid = {}", pid);

    if pid == 0 {
        return Ok(SyscallReturn::Return(ctx.process.sid() as _));
    }

    let process = pid_table::pid_table_mut()
        .get_process(pid)
        .ok_or_else(|| errno!(ESRCH, "the process to get the SID does not exist"))?;

    Ok(SyscallReturn::Return(process.sid() as _))
}
