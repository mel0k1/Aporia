#[cfg(not(target_arch = "loongarch64"))]
use ostd::arch::cpu::context::CpuException;
#[cfg(target_arch = "loongarch64")]
use ostd::arch::cpu::context::CpuExceptionInfo as CpuException;
use ostd::{arch::cpu::context::UserContext, task::Task};

use crate::{
    prelude::*,
    process::signal::signals::fault::FaultSignal,
    thread::kernel_thread::AsKernelThread,
    vm::vmar::{PageFaultInfo, Vmar},
};

pub(super) fn handle_exception(ctx: &Context, user_ctx: &UserContext, exception: CpuException) {
    debug!("handle exception: {:#x?}", exception);

    if let Ok(page_fault_info) = PageFaultInfo::try_from(&exception) {
        let user_space = ctx.user_space();
        let vmar = user_space.vmar();
        if handle_page_fault_from_vmar(vmar, &page_fault_info).is_ok() {
            return;
        }
    }

    generate_fault_signal(exception, ctx, user_ctx);
}

fn handle_page_fault_from_vmar(vmar: &Vmar, page_fault_info: &PageFaultInfo) -> Result<(), ()> {
    if let Err(e) = vmar.handle_page_fault(page_fault_info) {
        warn!(
            "page fault handler failed: info: {:#x?}, err: {:?}",
            page_fault_info, e
        );
        return Err(());
    }
    Ok(())
}

pub(crate) trait ToFaultSignal {
    fn to_fault_signal(&self, user_ctx: &UserContext) -> Option<FaultSignal>;
}

fn generate_fault_signal(exception: CpuException, ctx: &Context, user_ctx: &UserContext) {
    let Some(signal) = exception.to_fault_signal(user_ctx) else {
        panic!("`{:?}` cannot be handled via signals", exception);
    };
    ctx.posix_thread.enqueue_signal(Box::new(signal));
}

pub(super) fn page_fault_handler(info: &CpuException) -> Result<(), ()> {
    let task = Task::current().unwrap();
    let page_fault_info = info.try_into().unwrap();

    if let Some(thread_local) = task.as_thread_local() {
        if thread_local.is_page_fault_disabled() {
            return Err(());
        }

        let user_space = CurrentUserSpace::new(thread_local);
        handle_page_fault_from_vmar(user_space.vmar(), &page_fault_info)
    } else {
        let vmar = task
            .as_kernel_thread()
            .and_then(|kernel_thread| kernel_thread.vmar())
            .ok_or(())?;
        handle_page_fault_from_vmar(vmar, &page_fault_info)
    }
}
