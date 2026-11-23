use super::{Pgid, Pid};
use crate::{
    fs::file::file_table::{FileDesc, get_file_fast},
    prelude::*,
    process::PidFile,
};

#[derive(Clone, Debug)]
pub(crate) enum ProcessFilter {
    Any,
    WithPid(Pid),
    WithPgid(Pgid),
    WithPidfd(Arc<PidFile>),
}

impl ProcessFilter {
    pub(crate) fn from_which_and_id(which: u64, id: u32, ctx: &Context) -> Result<Self> {
        const P_ALL: u64 = 0;
        const P_PID: u64 = 1;
        const P_PGID: u64 = 2;
        const P_PIDFD: u64 = 3;

        match which {
            P_ALL => Ok(ProcessFilter::Any),
            P_PID => Ok(ProcessFilter::WithPid(id)),
            P_PGID => Ok(ProcessFilter::WithPgid(id)),
            P_PIDFD => {
                let fd = FileDesc::try_from(id.cast_signed())
                    .map_err(|_| errno!(EINVAL, "the pidfd is invalid"))?;
                let file = {
                    let mut file_table = ctx.thread_local.borrow_file_table_mut();
                    get_file_fast!(&mut file_table, fd).into_owned()
                };
                let pid_file = Arc::downcast(file)
                    .map_err(|_| errno!(EINVAL, "the file is not a PID file"))?;
                Ok(ProcessFilter::WithPidfd(pid_file))
            }
            _ => Err(errno!(EINVAL, "the process filter is invalid")),
        }
    }

    pub(crate) fn from_id(wait_pid: i32) -> Result<Self> {
        if wait_pid == i32::MIN {
            Err(errno!(ESRCH, "the target group does not exist"))
        } else if wait_pid < -1 {
            Ok(ProcessFilter::WithPgid((-wait_pid).cast_unsigned()))
        } else if wait_pid == -1 {
            Ok(ProcessFilter::Any)
        } else if wait_pid == 0 {
            let pgid = current!().pgid();
            Ok(ProcessFilter::WithPgid(pgid))
        } else {
            Ok(ProcessFilter::WithPid(wait_pid.cast_unsigned()))
        }
    }
}
