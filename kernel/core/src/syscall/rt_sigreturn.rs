use ostd::{
    arch::cpu::context::{FpuContext, UserContext},
    mm::VmIo,
    user::UserContextApi,
};

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{posix_thread::ContextPthreadAdminApi, signal::c_types::ucontext_t},
    syscall::sigaltstack::set_new_stack,
};

pub(super) fn sys_rt_sigreturn(ctx: &Context, user_ctx: &mut UserContext) -> Result<SyscallReturn> {
    let sig_context_addr = user_ctx.stack_pointer() as Vaddr;
    debug!(
        "sys_rt_sigreturn: sig_context_addr = {:#x}",
        sig_context_addr
    );

    let ucontext = ctx.user_space().read_val::<ucontext_t>(sig_context_addr)?;

    ucontext.uc_mcontext.copy_user_regs_to(user_ctx);

    let stack = ucontext.uc_stack;

    let _ = set_new_stack(stack, ctx, user_ctx.stack_pointer());

    let fpu_context_addr = cfg_select! {
        target_arch = "x86_64" => ucontext.uc_mcontext.fpu_context_addr(),
        any(
            target_arch = "riscv64",
            target_arch = "loongarch64",
            target_arch = "aarch64"
        ) => sig_context_addr + size_of::<ucontext_t>(),
        _ => compile_error!("unsupported target"),
    };
    let mut fpu_context = FpuContext::new();
    ctx.user_space()
        .read_bytes(fpu_context_addr, fpu_context.as_bytes_mut())?;
    ctx.thread_local.supp_user_context().fpu().set(fpu_context);

    let sig_mask = ucontext.uc_sigmask;
    ctx.set_sig_mask(sig_mask.into());

    Ok(SyscallReturn::NoReturn)
}
