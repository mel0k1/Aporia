use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{Pgid, Pid},
};

pub(super) fn sys_setpgid(pid: Pid, pgid: Pgid, ctx: &Context) -> Result<SyscallReturn> {
    let current = ctx.process.as_ref();

    if pid.cast_signed() < 0 || pgid.cast_signed() < 0 {
        return Err(errno!(EINVAL, "negative PIDs or PGIDs are not valid"));
    }

    let pid = if pid == 0 { current.pid() } else { pid };
    let pgid = if pgid == 0 { pid } else { pgid };

    debug!("pid = {}, pgid = {}", pid, pgid);

    current.move_process_to_group(pid, pgid)?;

    Ok(SyscallReturn::Return(0))
}
