use ostd::{arch::cpu::context::UserContext, mm::VmIo, task::Task};

use super::{
    AsPosixThread, ThreadLocal,
    futex::{FutexVisibility, futex_wake},
    ptrace::PtraceEvent,
    robust_list::wake_robust_futex,
};
use crate::{
    context::current_userspace,
    prelude::*,
    process::{
        TermStatus,
        exit::{drop_after, exit_process},
        pid_table,
        signal::{constants::SIGKILL, signals::kernel::KernelSignal},
        task_set::TaskSet,
    },
    thread::{AsThread, Tid},
};

pub(crate) fn do_exit(term_status: TermStatus, ctx: &Context, user_ctx: &mut UserContext) {
    exit_internal(term_status, false, ctx, user_ctx);
}

pub(crate) fn do_exit_group(term_status: TermStatus, ctx: &Context, user_ctx: &mut UserContext) {
    exit_internal(term_status, true, ctx, user_ctx);
}

fn exit_internal(
    term_status: TermStatus,
    is_exiting_group: bool,
    ctx: &Context,
    user_ctx: &mut UserContext,
) {
    let exit_code = term_status.as_u32();
    ctx.posix_thread
        .ptrace_may_stop_on(PtraceEvent::Exit(exit_code), ctx, user_ctx);

    let current_task = Task::current().unwrap();
    let current_thread = current_task.as_thread().unwrap();
    let posix_thread = current_thread.as_posix_thread().unwrap();
    let thread_local = current_task.as_thread_local().unwrap();
    let posix_process = posix_thread.process();

    let is_last_thread = {
        let mut tasks = posix_process.tasks().lock();
        let has_exited_group = tasks.has_exited_group();
        let in_evecve = tasks.in_execve();

        if is_exiting_group && !has_exited_group && !in_evecve {
            sigkill_other_threads(&current_task, &tasks);
            tasks.set_exited_group();
        }

        if !has_exited_group && !in_evecve {
            posix_process.status().set_exit_code(exit_code);
        }

        posix_thread.set_exit_code(exit_code);

        if current_thread.is_exited() {
            return;
        }
        current_thread.exit();

        tasks.remove_exited(&current_task)
    };

    posix_thread.clear_tracees();

    if let Some(tracer) = posix_thread.tracer() {
        let tracer = tracer.as_posix_thread().unwrap();
        tracer.process().children_wait_queue().wake_all();
    }

    wake_clear_ctid(thread_local);

    wake_robust_list(thread_local, posix_thread.tid());

    if posix_thread.tid() != posix_process.pid() {
        pid_table::pid_table_mut().remove_thread(posix_thread.tid());
    }

    drop_after!(posix_thread.file_table().lock().take());
    drop_after!(posix_thread.fs.write().take());
    drop_after!(posix_thread.ns_proxy().lock().take());

    drop_after!(thread_local.vmar().borrow_mut().take());
    drop_after!(thread_local.remove_file_table());
    drop_after!(thread_local.remove_fs());
    drop_after!(thread_local.remove_ns_proxy());

    if is_last_thread {
        exit_process(&posix_process);
    }
}

pub(in crate::process) fn sigkill_other_threads(current_task: &Task, task_set: &TaskSet) {
    debug_assert!(
        task_set
            .as_slice()
            .iter()
            .any(|task| core::ptr::eq(current_task, task.as_ref()))
    );

    for task in task_set.as_slice() {
        if core::ptr::eq(current_task, task.as_ref()) {
            continue;
        }
        task.as_posix_thread()
            .unwrap()
            .enqueue_signal(Box::new(KernelSignal::new(SIGKILL)));
    }
}

fn wake_clear_ctid(thread_local: &ThreadLocal) {
    let clear_ctid = thread_local.clear_child_tid().get();

    if clear_ctid == 0 {
        return;
    }

    let _ = current_userspace!()
        .write_val(clear_ctid, &0u32)
        .inspect_err(|err| debug!("exit: cannot clear the child TID: {:?}", err));
    let _ = futex_wake(clear_ctid, 1, FutexVisibility::Shared)
        .inspect_err(|err| debug!("exit: cannot wake the futex on the child TID: {:?}", err));

    thread_local.clear_child_tid().set(0);
}

fn wake_robust_list(thread_local: &ThreadLocal, tid: Tid) {
    let list_head = match thread_local.robust_list().borrow_mut().take() {
        Some(robust_list_head) => robust_list_head,
        None => return,
    };

    debug!("exit: wake up the robust list: {:?}", list_head);
    for futex_addr in list_head.futexes() {
        if let Err(err) = wake_robust_futex(futex_addr, tid) {
            debug!(
                "exit: cannot wake the robust futex at {:#x}: {:?}",
                futex_addr, err
            );
            return;
        }
    }
}
