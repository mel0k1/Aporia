use ostd::mm::VmIo;

use crate::{
    fs::file::file_table::{RawFileDesc, get_file_fast},
    prelude::*,
    process::{
        Pgid, Pid, PidFile, kill, kill_group,
        posix_thread::AsPosixThread,
        signal::{
            c_types::siginfo_t,
            constants::SI_TKILL,
            sig_num::SigNum,
            signals::{Signal, raw::RawSignal, user::UserSignal},
        },
        tgkill,
    },
    syscall::SyscallReturn,
    thread::Tid,
};

pub(super) fn sys_pidfd_send_signal(
    pidfd: RawFileDesc,
    sig_num: u64,
    info_ptr: Vaddr,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let flags = PidfdSendSignalFlags::try_from(flags)?;
    let sig_num = if sig_num == 0 {
        None
    } else {
        Some(SigNum::try_from(sig_num as u8)?)
    };
    debug!(
        "pidfd={}, info_ptr={:#x}, flags={:?}",
        pidfd, info_ptr, flags
    );

    let target = get_target_from_pidfd(pidfd, flags, ctx)?;

    let signal = if let Some(sig_num) = sig_num {
        let siginfo = read_siginfo_from_user(info_ptr, sig_num, ctx)?;

        if info_ptr != 0 {
            let is_self = match &target {
                SignalTarget::Thread { tid, tgid: _ } => *tid == ctx.posix_thread.tid(),
                SignalTarget::Process { pid } => *pid == ctx.posix_thread.tid(),
                SignalTarget::ProcessGroup { pgid: _ } => false,
            };

            if !is_self && (siginfo.si_code >= 0 || siginfo.si_code == SI_TKILL) {
                return Err(errno!(
                    EPERM,
                    "signals with custom code can only be sent to the current thread/process"
                ));
            }
        }

        Some(RawSignal::new(siginfo))
    } else {
        None
    };

    match target {
        SignalTarget::Thread { tid, tgid } => {
            let signal = signal.map(|s| Box::new(s) as Box<dyn Signal>);
            tgkill(tid, Some(tgid), signal, ctx)?;
        }
        SignalTarget::Process { pid } => {
            let signal = signal.map(|s| Box::new(s) as Box<dyn Signal>);
            kill(pid, signal, ctx)?;
        }
        SignalTarget::ProcessGroup { pgid } => {
            kill_group(pgid, signal, ctx)?;
        }
    }

    Ok(SyscallReturn::Return(0))
}

fn read_siginfo_from_user(info_ptr: Vaddr, sig_num: SigNum, ctx: &Context) -> Result<siginfo_t> {
    if info_ptr != 0 {
        let si = ctx.user_space().read_val::<siginfo_t>(info_ptr)?;
        if si.si_signo != sig_num.as_u8() as i32 {
            return Err(errno!(
                EINVAL,
                "`siginfo.si_signo` does not match the specified signal number"
            ));
        }
        Ok(si)
    } else {
        Ok(UserSignal::new_kill(sig_num, ctx).to_info())
    }
}

fn get_target_from_pidfd(
    pidfd: RawFileDesc,
    flags: PidfdSendSignalFlags,
    ctx: &Context,
) -> Result<SignalTarget> {
    let thread_target = |tid: Tid, tgid: Pid| SignalTarget::Thread { tid, tgid };
    let process_target = |pid: Pid| SignalTarget::Process { pid };
    let group_target = |pgid: Pgid| SignalTarget::ProcessGroup { pgid };

    let target = match pidfd {
        PIDFD_SELF_THREAD => match flags {
            PidfdSendSignalFlags::Default | PidfdSendSignalFlags::Thread => {
                thread_target(ctx.posix_thread.tid(), ctx.process.pid())
            }
            PidfdSendSignalFlags::ThreadGroup => process_target(ctx.process.pid()),
            PidfdSendSignalFlags::ProcessGroup => group_target(ctx.posix_thread.tid()),
        },
        PIDFD_SELF_THREAD_GROUP => match flags {
            PidfdSendSignalFlags::Default | PidfdSendSignalFlags::ThreadGroup => {
                process_target(ctx.process.pid())
            }
            PidfdSendSignalFlags::Thread => thread_target(
                ctx.process.main_thread().as_posix_thread().unwrap().tid(),
                ctx.process.pid(),
            ),
            PidfdSendSignalFlags::ProcessGroup => group_target(ctx.process.pid()),
        },
        _ => {
            let mut file_table = ctx.thread_local.borrow_file_table_mut();
            let file = get_file_fast!(&mut file_table, pidfd.try_into()?);

            let Some(pid_file) = file.downcast_ref::<PidFile>() else {
                return Err(errno!(EBADF, "the file is not a PID file"));
            };

            let Some(process) = pid_file.process_opt() else {
                return Err(errno!(ESRCH, "the target process has been reaped"));
            };

            match flags {
                PidfdSendSignalFlags::Default => process_target(process.pid()),
                PidfdSendSignalFlags::Thread => thread_target(
                    process.main_thread().as_posix_thread().unwrap().tid(),
                    process.pid(),
                ),
                PidfdSendSignalFlags::ThreadGroup => process_target(process.pid()),
                PidfdSendSignalFlags::ProcessGroup => group_target(process.pid()),
            }
        }
    };

    Ok(target)
}

enum SignalTarget {
    Thread { tid: Tid, tgid: Pid },
    Process { pid: Pid },
    ProcessGroup { pgid: Pgid },
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
enum PidfdSendSignalFlags {
    Default = 0x0,
    Thread = 0x1,
    ThreadGroup = 0x2,
    ProcessGroup = 0x4,
}

const PIDFD_SELF_THREAD: i32 = -10000;
const PIDFD_SELF_THREAD_GROUP: i32 = -10001;
