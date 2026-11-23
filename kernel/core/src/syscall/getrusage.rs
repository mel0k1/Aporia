use int_to_c_enum::TryFromInt;
use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{prelude::*, time::timeval_t};

#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, TryFromInt)]
enum RusageTarget {
    ForSelf = 0,
    Children = -1,
    Thread = 1,
}

pub(super) fn sys_getrusage(
    target: i32,
    rusage_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let rusage_target = RusageTarget::try_from(target)?;

    debug!(
        "target = {:?}, rusage_addr = {}",
        rusage_target, rusage_addr,
    );

    if rusage_addr != 0 {
        let rusage = match rusage_target {
            RusageTarget::ForSelf => {
                let process = ctx.process.as_ref();
                rusage_t {
                    ru_utime: process.prof_clock().user_clock().read_time().into(),
                    ru_stime: process.prof_clock().kernel_clock().read_time().into(),
                    ..Default::default()
                }
            }
            RusageTarget::Thread => {
                let posix_thread = ctx.posix_thread;
                rusage_t {
                    ru_utime: posix_thread.prof_clock().user_clock().read_time().into(),
                    ru_stime: posix_thread.prof_clock().kernel_clock().read_time().into(),
                    ..Default::default()
                }
            }
            RusageTarget::Children => {
                let (user_time, kernel_time) = ctx.process.reaped_children_stats().lock().get();
                rusage_t {
                    ru_utime: user_time.into(),
                    ru_stime: kernel_time.into(),
                    ..Default::default()
                }
            }
        };

        ctx.user_space().write_val(rusage_addr, &rusage)?;
    }

    Ok(SyscallReturn::Return(0))
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct rusage_t {
    pub ru_utime: timeval_t,
    pub ru_stime: timeval_t,
    pub ru_maxrss: u64,
    pub ru_ixrss: u64,
    pub ru_idrss: u64,
    pub ru_isrss: u64,
    pub ru_minflt: u64,
    pub ru_majflt: u64,
    pub ru_nswap: u64,
    pub ru_inblock: u64,
    pub ru_oublock: u64,
    pub ru_msgsnd: u64,
    pub ru_msgrcv: u64,
    pub ru_nsignals: u64,
    pub ru_nvcsw: u64,
    pub ru_nivcsw: u64,
}
