pub(crate) mod c_types;
pub(crate) mod constants;
mod pause;
mod pending;
mod poll;
pub(crate) mod sig_action;
pub(crate) mod sig_disposition;
pub(crate) mod sig_mask;
pub(crate) mod sig_num;
pub(crate) mod sig_queues;
mod sig_stack;
pub(crate) mod signals;

use align_ext::AlignExt;
use c_types::{siginfo_t, ucontext_t};
use constants::SIGSEGV;
use ostd::{
    arch::cpu::context::{FpuContext, UserContext},
    mm::VmIo,
    user::UserContextApi,
};
pub(crate) use pause::{Pause, PauseReason, with_sigmask_changed};
pub(crate) use pending::{DequeuedSignal, HandlePendingSignal};
pub(crate) use poll::{PollAdaptor, Pollee, Poller};
pub use poll::{PollHandle, Pollable};
use sig_action::{SigAction, SigActionFlags, SigDefaultAction};
use sig_mask::SigMask;
use sig_num::SigNum;
pub(crate) use sig_stack::{SigStack, SigStackFlags, SigStackStatus};

use super::posix_thread::ThreadLocal;
use crate::{
    cpu::LinuxAbi,
    prelude::*,
    process::{
        TermStatus,
        posix_thread::{ContextPthreadAdminApi, do_exit_group, ptrace::PtraceStopResult},
        signal::{c_types::stack_t, constants::SIGKILL},
    },
};

pub(crate) trait SignalContext {
    /// Sets signal handler arguments.
    fn set_arguments(&mut self, sig_num: SigNum, siginfo_addr: usize, ucontext_addr: usize);
}

/// Handles a pending signal for the current process.
pub(crate) fn handle_pending_signal(user_ctx: &mut UserContext, ctx: &Context) {
    let syscall_restart = if let Some(orig_syscall_ret) = ctx.thread_local.orig_syscall_ret()
        && user_ctx.syscall_ret() == -(Errno::ERESTARTSYS as i32) as usize
    {
        user_ctx.set_syscall_ret(-(Errno::EINTR as i32) as usize);
        Some(orig_syscall_ret)
    } else {
        None
    };

    let mut restore_sig_mask = ctx
        .thread_local
        .sig_mask_saved()
        .take()
        .map(|mask| RestoreSigMaskGuard { ctx, mask });

    let (dequeued, sig_action) = if let Some(dequeued_signal) = dequeue_pending_signal(ctx) {
        dequeued_signal
    } else {
        if restore_sig_mask.is_none() {
            return;
        }

        let _ = restore_sig_mask.take();

        if let Some(dequeued_signal) = dequeue_pending_signal(ctx) {
            dequeued_signal
        } else {
            return;
        }
    };

    let (signal, sig_action) = if dequeued.num() != SIGKILL {
        match ctx.posix_thread.ptrace_stop(dequeued, ctx, user_ctx) {
            PtraceStopResult::Continued(Some(dequeued)) => {
                let sig_num = dequeued.num();
                if ctx.posix_thread.sig_mask().contains(sig_num) {
                    requeue_signal(ctx, dequeued);
                    return;
                }

                (dequeued.unwrap(), get_sig_action(ctx, sig_num))
            }
            PtraceStopResult::Continued(None) => return,
            PtraceStopResult::Interrupted => match dequeue_pending_signal(ctx) {
                Some((dequeued, action)) if dequeued.num() == SIGKILL => {
                    (dequeued.unwrap(), action)
                }

                _ => return,
            },
            PtraceStopResult::NotTraced(dequeued) => (dequeued.unwrap().unwrap(), sig_action),
        }
    } else {
        (dequeued.unwrap(), sig_action)
    };

    let sig_num = signal.num();
    match sig_action {
        SigAction::Ign => {
            debug!("Ignore signal {:?}", sig_num);
        }
        SigAction::User {
            handler_addr,
            flags,
            restorer_addr,
            mask,
        } => {
            if flags.contains(SigActionFlags::SA_RESETHAND) {
                let sig_dispositions = ctx.process.sig_dispositions().lock();
                let mut sig_dispositions = sig_dispositions.lock();
                sig_dispositions.set_default(sig_num);
            }

            if let Some(orig_syscall_ret) = syscall_restart
                && flags.contains(SigActionFlags::SA_RESTART)
            {
                #[cfg(target_arch = "x86_64")]
                const SYSCALL_INSTR_LEN: usize = 2;
                #[cfg(target_arch = "riscv64")]
                const SYSCALL_INSTR_LEN: usize = 4;
                #[cfg(target_arch = "loongarch64")]
                const SYSCALL_INSTR_LEN: usize = 4;
                #[cfg(target_arch = "aarch64")]
                const SYSCALL_INSTR_LEN: usize = 4;

                user_ctx.set_syscall_ret(orig_syscall_ret);
                user_ctx
                    .set_instruction_pointer(user_ctx.instruction_pointer() - SYSCALL_INSTR_LEN);
            }

            if let Err(e) = handle_user_signal(
                ctx,
                sig_num,
                handler_addr,
                flags,
                restorer_addr,
                mask,
                restore_sig_mask.map(RestoreSigMaskGuard::into_mask),
                user_ctx,
                signal.to_info(),
            ) {
                debug!("Failed to handle user signal: {:?}", e);

                do_exit_group(TermStatus::Killed(SIGSEGV), ctx, user_ctx);
            }
        }
        SigAction::Dfl if ctx.process.is_init_process() => {}
        SigAction::Dfl => {
            let sig_default_action = SigDefaultAction::from_signum(sig_num);
            debug!("sig_default_action = {:?}", sig_default_action);

            match sig_default_action {
                SigDefaultAction::Core | SigDefaultAction::Term => {
                    warn!(
                        "PID {}: terminating on signal {}",
                        ctx.process.pid(),
                        sig_num.sig_name()
                    );

                    do_exit_group(TermStatus::Killed(sig_num), ctx, user_ctx);
                }
                SigDefaultAction::Ign => {}
                SigDefaultAction::Stop => ctx.process.stop(sig_num),
                SigDefaultAction::Cont => ctx.process.resume(),
            }
        }
    }
}

/// A guard that restores the signal mask on drop.
struct RestoreSigMaskGuard<'a> {
    ctx: &'a Context<'a>,
    mask: SigMask,
}

impl RestoreSigMaskGuard<'_> {
    /// Forgets the guard and returns the signal mask to restore.
    fn into_mask(self) -> SigMask {
        let mask = self.mask;

        let _ = self.mask;

        core::mem::forget(self);

        mask
    }
}

impl Drop for RestoreSigMaskGuard<'_> {
    fn drop(&mut self) {
        self.ctx.set_sig_mask(self.mask);
    }
}

fn dequeue_pending_signal(ctx: &Context) -> Option<(DequeuedSignal, SigAction)> {
    let posix_thread = ctx.posix_thread;

    let sig_dispositions = ctx.process.sig_dispositions().lock();
    let sig_dispositions = sig_dispositions.lock();

    let sig_mask = posix_thread.sig_mask();
    let (signal, sig_num, sig_action) = loop {
        let signal = ctx.dequeue_signal(&sig_mask)?;
        let sig_num = signal.num();
        let sig_action = sig_dispositions.get(sig_num);
        if sig_action.will_ignore(sig_num) && !posix_thread.is_traced() {
            continue;
        }

        break (signal, sig_num, sig_action);
    };

    debug!(
        "sig_num = {:?}, sig_name = {}, sig_action = {:#x?}",
        sig_num,
        sig_num.sig_name(),
        sig_action
    );

    Some((signal, sig_action))
}

/// Re-queue the ptrace-captured signal.
///
/// This appends to the tail of the origin queue,
/// which (a) loses same-number RT-signal send-order FIFO and
/// (b) allows newer lower-numbered standard signals queued
/// during the ptrace stop to preempt this signal.
/// This is intentionally consistent with Linux's behaviors.
fn requeue_signal(ctx: &Context, signal: DequeuedSignal) {
    match signal {
        DequeuedSignal::FromProcess(signal) => ctx.process.enqueue_signal(signal),
        DequeuedSignal::FromThread(signal) => ctx.posix_thread.enqueue_signal(signal),
    }
}

fn get_sig_action(ctx: &Context, sig_num: SigNum) -> SigAction {
    let sig_dispositions = ctx.process.sig_dispositions().lock();
    let sig_dispositions = sig_dispositions.lock();
    sig_dispositions.get(sig_num)
}

#[expect(clippy::too_many_arguments)]
pub(crate) fn handle_user_signal(
    ctx: &Context,
    sig_num: SigNum,
    handler_addr: Vaddr,
    flags: SigActionFlags,
    restorer_addr: Vaddr,
    mut mask: SigMask,
    mask_to_restore: Option<SigMask>,
    user_ctx: &mut UserContext,
    sig_info: siginfo_t,
) -> Result<()> {
    debug!("sig_num = {:?}, signame = {}", sig_num, sig_num.sig_name());
    debug!("handler_addr = 0x{:x}", handler_addr);
    debug!("flags = {:?}", flags);
    debug!("restorer_addr = 0x{:x}", restorer_addr);
    debug!("mask = {:?}, mask_to_restore = {:?}", mask, mask_to_restore);

    if flags.contains_unsupported_flag() {
        warn!("Unsupported signal flags: {:?}", flags);
    }

    if !flags.contains(SigActionFlags::SA_NODEFER) {
        mask += sig_num;
    }

    let old_mask = ctx.posix_thread.sig_mask();
    let mask_to_restore = mask_to_restore.unwrap_or(old_mask);
    ctx.set_sig_mask(old_mask + mask);

    let mut stack_pointer = if let Some(sp) =
        use_alternate_signal_stack(flags, ctx.thread_local, user_ctx.stack_pointer())
    {
        sp as u64
    } else {
        let sp = user_ctx.stack_pointer() as u64;

        sp.wrapping_sub(128)
    };

    let user_space = ctx.user_space();

    stack_pointer = alloc_aligned_in_user_stack(
        stack_pointer,
        size_of::<siginfo_t>(),
        align_of::<siginfo_t>(),
    );
    user_space.write_val(stack_pointer as _, &sig_info)?;
    let siginfo_addr = stack_pointer;

    let uc_stack = {
        let mut sig_stack = ctx.thread_local.sig_stack().borrow_mut();
        let stack = stack_t::from(&*sig_stack);

        if sig_stack.flags().contains(SigStackFlags::SS_AUTODISARM) {
            sig_stack.reset();
        }

        stack
    };

    let mut ucontext = ucontext_t {
        uc_stack,
        uc_sigmask: mask_to_restore.into(),
        ..Default::default()
    };

    ucontext.uc_mcontext.copy_user_regs_from(user_ctx);

    let supp = ctx.thread_local.supp_user_context();
    let fpu_context = supp.fpu().get();
    let fpu_context_bytes = fpu_context.as_bytes();
    supp.fpu().set(FpuContext::new());

    cfg_select! {
        target_arch = "x86_64" => {
            let fpu_context_addr =
                alloc_aligned_in_user_stack(stack_pointer, fpu_context_bytes.len(), 64);
            let ucontext_addr = alloc_aligned_in_user_stack(
                fpu_context_addr,
                size_of::<ucontext_t>(),
                align_of::<ucontext_t>(),
            );
            ucontext
                .uc_mcontext
                .set_fpu_context_addr(fpu_context_addr as _);

            const UC_FP_XSTATE: u64 = 1 << 0;
            ucontext.uc_flags = UC_FP_XSTATE;
        }
        target_arch = "riscv64" => {
            const FP_STATE_SIZE: usize =
                size_of::<ostd::arch::cpu::context::QFpuContext>() + 3 * size_of::<u32>();

            let ucontext_addr = alloc_aligned_in_user_stack(
                stack_pointer,
                size_of::<ucontext_t>() + FP_STATE_SIZE,
                align_of::<ucontext_t>(),
            );
            let fpu_context_addr = (ucontext_addr as usize) + size_of::<ucontext_t>();
        }
        any(target_arch = "loongarch64", target_arch = "aarch64") => {
            let ucontext_addr = alloc_aligned_in_user_stack(
                stack_pointer,
                size_of::<ucontext_t>() + fpu_context_bytes.len(),
                align_of::<ucontext_t>(),
            );

            let fpu_context_addr = (ucontext_addr as usize) + size_of::<ucontext_t>();
        }
        _ => {
            compile_error!("unsupported target");
        }
    }

    user_space.write_bytes(fpu_context_addr as _, fpu_context_bytes)?;
    user_space.write_val(ucontext_addr as _, &ucontext)?;

    stack_pointer = ucontext_addr.align_down(16);

    let retaddr = if flags.contains(SigActionFlags::SA_RESTORER) {
        restorer_addr
    } else {
        cfg_select! {
            any(target_arch = "riscv64", target_arch = "aarch64") => {
                ctx.user_space().vmar().process_vm().vdso_base()
                    + crate::vdso::__VDSO_RT_SIGRETURN_OFFSET
            }
            _ => {
                return Err(errno!(
                    EINVAL,
                    "this architecture currently requires SA_RESTORER"
                ));
            }
        }
    };
    cfg_select! {
        target_arch = "x86_64" => {
            stack_pointer = write_u64_to_user_stack(stack_pointer, retaddr as u64)?;
        }
        any(target_arch = "riscv64", target_arch = "loongarch64") => {
            user_ctx.set_ra(retaddr);
        }
        target_arch = "aarch64" => {
            user_ctx.set_x30(retaddr);
        }
        _ => {
            compile_error!("unsupported target");
        }
    }

    debug!(
        "Before calling to signal handler: stack_pointer = 0x{:x}",
        stack_pointer
    );

    user_ctx.set_instruction_pointer(handler_addr as _);
    user_ctx.set_stack_pointer(stack_pointer as usize);

    if flags.contains(SigActionFlags::SA_SIGINFO) {
        user_ctx.set_arguments(sig_num, siginfo_addr as usize, ucontext_addr as usize);
    } else {
        user_ctx.set_arguments(sig_num, 0, 0);
    }

    cfg_select! {
        target_arch = "x86_64" => {
            const X86_RFLAGS_DF: usize = 1 << 10;
            user_ctx.set_rflags(user_ctx.rflags() & !X86_RFLAGS_DF);
        }
        _ => {}
    }

    Ok(())
}

/// Uses the alternate signal stack configured via `sigaltstack`.
///
/// If the current stack pointer `sp` is already within the alternate signal stack
/// or if the stack is disabled, this function returns `None`.
/// Otherwise, it returns the starting stack pointer of the alternate signal stack.
fn use_alternate_signal_stack(
    flags: SigActionFlags,
    thread_local: &ThreadLocal,
    sp: usize,
) -> Option<usize> {
    if !flags.contains(SigActionFlags::SA_ONSTACK) {
        return None;
    }

    let sig_stack = thread_local.sig_stack().borrow();

    if sig_stack.active_status(sp) != SigStackStatus::Inactive {
        return None;
    }

    Some(sig_stack.base() + sig_stack.size())
}

/// Writes a `u64` integer to the user's stack.
#[cfg(target_arch = "x86_64")]
fn write_u64_to_user_stack(sp: u64, value: u64) -> Result<u64> {
    use crate::context::current_userspace;

    let sp = sp.wrapping_sub(size_of::<u64>() as u64);
    current_userspace!().write_val(sp as _, &value)?;
    Ok(sp)
}

/// Allocates `size` bytes on the user's stack.
///
/// The allocation will be aligned to `align`, which must be a power of two.
fn alloc_aligned_in_user_stack(sp: u64, size: usize, align: usize) -> u64 {
    sp.wrapping_sub(size as u64).align_down(align as u64)
}
