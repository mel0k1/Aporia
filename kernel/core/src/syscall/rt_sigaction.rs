use ostd::mm::VmIo;

use super::{SyscallReturn, rt_sigprocmask::RequireFullSize};
use crate::{
    prelude::*,
    process::{
        posix_thread::AsPosixThread,
        signal::{
            HandlePendingSignal,
            c_types::sigaction_t,
            constants::{SIGKILL, SIGSTOP},
            sig_action::SigAction,
            sig_mask::SigSet,
            sig_num::SigNum,
        },
    },
};

pub(super) fn sys_rt_sigaction(
    sig_num: u8,
    sig_action_addr: Vaddr,
    old_sig_action_addr: Vaddr,
    sigset_size: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let sig_num = SigNum::try_from(sig_num)?;
    debug!(
        "signal = {}, sig_action_addr = 0x{:x}, old_sig_action_addr = 0x{:x}, sigset_size = {}",
        sig_num.sig_name(),
        sig_action_addr,
        old_sig_action_addr,
        sigset_size
    );

    RequireFullSize::new(sigset_size)?;

    let sig_dispositions = ctx.process.sig_dispositions().lock();
    let mut sig_dispositions = sig_dispositions.lock();

    let old_action = if sig_action_addr != 0 {
        if sig_num == SIGKILL || sig_num == SIGSTOP {
            return Err(errno!(
                EINVAL,
                "cannot set a new signal action for SIGKILL and SIGSTOP"
            ));
        }

        let sig_action_c = ctx.user_space().read_val::<sigaction_t>(sig_action_addr)?;
        let sig_action = SigAction::from(sig_action_c);
        debug!("sig action = {:?}", sig_action);
        if sig_action.will_ignore(sig_num) {
            discard_signals_if_ignored(ctx, sig_num);
        }

        sig_dispositions.set(sig_num, sig_action)?
    } else {
        sig_dispositions.get(sig_num)
    };

    if old_sig_action_addr != 0 {
        let old_action_c = old_action.as_c_type();
        ctx.user_space()
            .write_val(old_sig_action_addr, &old_action_c)?;
    }

    Ok(SyscallReturn::Return(0))
}

fn discard_signals_if_ignored(ctx: &Context, signum: SigNum) {
    let mask = SigSet::new_full() - signum;

    for task in ctx.process.tasks().lock().as_slice() {
        let Some(posix_thread) = task.as_posix_thread() else {
            continue;
        };

        while posix_thread.dequeue_signal(&mask).is_some() {}
    }
}
