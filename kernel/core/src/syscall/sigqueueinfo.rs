use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{
        Pid, kill,
        signal::{
            c_types::siginfo_t,
            sig_num::SigNum,
            signals::{Signal, user::UserSignal},
        },
        tgkill,
    },
    thread::Tid,
};

pub(super) fn sys_rt_sigqueueinfo(
    tgid: Pid,
    sig: u8,
    uinfo_ptr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let sig_num = prepare_sigqueue_signal(tgid, sig, uinfo_ptr, ctx)?;
    let signal = sig_num.map(|(sig_num, sigval)| {
        Box::new(UserSignal::new_sigqueue(sig_num, ctx, sigval)) as Box<dyn Signal>
    });
    kill(tgid, signal, ctx)?;
    Ok(SyscallReturn::Return(0))
}

pub(super) fn sys_rt_tgsigqueueinfo(
    tgid: Pid,
    tid: Tid,
    sig: u8,
    uinfo_ptr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    if tgid.cast_signed() <= 0 || tid.cast_signed() <= 0 {
        return Err(errno!(EINVAL, "non-positive TGIDs or TIDs are not valid"));
    }

    let sig_num = prepare_sigqueue_signal(tgid, sig, uinfo_ptr, ctx)?;
    let signal = sig_num.map(|(sig_num, sigval)| {
        Box::new(UserSignal::new_tkill_with_sigval(sig_num, ctx, sigval)) as Box<dyn Signal>
    });
    tgkill(tid, Some(tgid), signal, ctx)?;
    Ok(SyscallReturn::Return(0))
}

fn prepare_sigqueue_signal(
    pid: Pid,
    sig: u8,
    uinfo_ptr: Vaddr,
    ctx: &Context,
) -> Result<Option<(SigNum, Vaddr)>> {
    debug!("pid = {}, sig = {}, uinfo_ptr = {:#x}", pid, sig, uinfo_ptr);

    if pid.cast_signed() <= 0 {
        return Err(errno!(EINVAL, "non-positive PIDs are not valid"));
    }

    let sig_num = if sig == 0 {
        None
    } else {
        Some(SigNum::try_from(sig)?)
    };

    let uinfo: siginfo_t = ctx.user_space().read_val(uinfo_ptr)?;

    if uinfo.si_signo != sig as i32 {
        return Err(errno!(EINVAL, "si_signo does not match sig"));
    }

    if uinfo.si_code < 0 {
        return Err(errno!(
            EPERM,
            "user-provided siginfo cannot claim kernel-generated codes"
        ));
    }

    let sigval = uinfo.sigval();

    Ok(sig_num.map(|sig_num| (sig_num, sigval)))
}
