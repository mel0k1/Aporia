use aster_rights::ReadWriteOp;
#[cfg(target_arch = "x86_64")]
use ostd::arch::cpu::context::{FsBase, GsBase};
use ostd::{
    arch::cpu::context::{FpuContext, UserContext},
    mm::VmIo,
    sync::Waiter,
    user::UserContextApi,
};

use super::process_vm::activate_vmar;
use crate::{
    fs::vfs::path::{Path, PerMountFlags},
    prelude::*,
    process::{
        ContextUnshareAdminApi, Credentials, Process,
        credentials::{ExecCred, FileCapabilities},
        pid_table,
        posix_thread::{
            AsPosixThread, ContextPthreadAdminApi, ThreadLocal, ThreadName, ptrace::PtraceEvent,
            sigkill_other_threads,
        },
        process_vm::{MAX_LEN_STRING_ARG, MAX_NR_STRING_ARGS, ProcessVm},
        program_loader::{ProgramToLoad, UndetectedExecutable, elf::ElfLoadInfo},
        signal::{
            HandlePendingSignal, PauseReason, SigStack,
            constants::{SIGCHLD, SIGKILL},
            signals::kernel::KernelSignal,
        },
    },
    vm::vmar::VmarHandle,
};

pub(crate) fn do_execve(
    executable: UndetectedExecutable,
    thread_name: ThreadName,
    argv_ptr_ptr: Vaddr,
    envp_ptr_ptr: Vaddr,
    ctx: &Context,
    user_context: &mut UserContext,
) -> Result<()> {
    let argv = read_cstring_vec(argv_ptr_ptr, MAX_NR_STRING_ARGS, MAX_LEN_STRING_ARG, ctx)?;
    let envp = read_cstring_vec(envp_ptr_ptr, MAX_NR_STRING_ARGS, MAX_LEN_STRING_ARG, ctx)?;

    let fs_ref = ctx.thread_local.borrow_fs();
    let path_resolver = fs_ref.resolver().read();

    debug!(
        "file path: {:?}, argv = {:?}, envp = {:?}",
        path_resolver.make_abs_path(executable.path()).into_string(),
        argv,
        envp
    );

    let executable_path = executable.path().clone();
    let program_to_load = ProgramToLoad::from_executable(executable, &path_resolver, argv, envp)?;
    let exec_cred = prepare_exec_cred(program_to_load.elf_path(), ctx)?;

    let new_vmar = VmarHandle::new(ProcessVm::new(executable_path.clone()));
    let elf_load_info = program_to_load.load_to_vmar(&new_vmar, &path_resolver)?;

    let mut task_set = ctx.process.tasks().lock();
    if task_set.has_exited_group() || task_set.in_execve() {
        return Err(errno!(
            EAGAIN,
            "the process has exited or has already executed a new program"
        ));
    }
    task_set.start_execve();

    sigkill_other_threads(ctx.task, &task_set);
    drop(task_set);

    let former_tid = ctx.posix_thread.tid();

    let res = do_execve_no_return(
        ctx,
        user_context,
        thread_name,
        new_vmar,
        &elf_load_info,
        exec_cred,
    );

    if res.is_ok() {
        ctx.posix_thread
            .ptrace_may_stop_on(PtraceEvent::Exec(former_tid), ctx, user_context);
    } else {
        ctx.posix_thread
            .enqueue_signal(Box::new(KernelSignal::new(SIGKILL)));
    }

    ctx.process.tasks().lock().finish_execve();

    res
}

fn read_cstring_vec(
    array_ptr: Vaddr,
    max_string_number: usize,
    max_string_len: usize,
    ctx: &Context,
) -> Result<Vec<CString>> {
    if array_ptr == 0 {
        return Ok(Vec::new());
    }

    let mut res = Vec::new();
    let mut read_addr = array_ptr;

    let user_space = ctx.user_space();
    for _ in 0..max_string_number {
        let cstring_ptr = user_space.read_val::<usize>(read_addr)?;
        read_addr += 8;

        if cstring_ptr == 0 {
            return Ok(res);
        }

        let cstring = user_space
            .read_cstring(cstring_ptr, max_string_len)
            .map_err(|err| {
                if err.error() == Errno::ENAMETOOLONG {
                    errno!(E2BIG, "there are too many bytes in the argument")
                } else {
                    err
                }
            })?;
        res.push(cstring);
    }

    Err(errno!(E2BIG, "there are too many arguments"))
}

/// Prepares credential changes.
fn prepare_exec_cred(elf_path: &Path, ctx: &Context) -> Result<ExecCred> {
    let credentials = ctx.posix_thread.credentials();
    if elf_path
        .mount_node()
        .flags()
        .contains(PerMountFlags::NOSUID)
    {
        return credentials.prepare_exec_cred(None, None, None);
    }

    let current_user_ns_owner_uid = ctx.thread_local.borrow_user_ns().owner_uid()?;
    let file_capabilities =
        FileCapabilities::read_from_inode(elf_path.inode())?.filter(|file_capabilities| {
            file_capabilities
                .root_uid()
                .map_or(current_user_ns_owner_uid.is_root(), |root_uid| {
                    root_uid == current_user_ns_owner_uid
                })
        });
    let elf_mode = elf_path.mode()?;
    let setuid = if elf_mode.has_set_uid() {
        Some(elf_path.owner()?)
    } else {
        None
    };
    let setgid = if elf_mode.has_set_gid() {
        Some(elf_path.group()?)
    } else {
        None
    };

    credentials.prepare_exec_cred(file_capabilities, setuid, setgid)
}

fn do_execve_no_return(
    ctx: &Context,
    user_context: &mut UserContext,
    thread_name: ThreadName,
    new_vmar: VmarHandle,
    elf_load_info: &ElfLoadInfo,
    exec_cred: ExecCred,
) -> Result<()> {
    let Context {
        process,
        thread_local,
        posix_thread,
        ..
    } = ctx;

    wait_other_threads_exit(ctx)?;
    make_current_main_thread(ctx);

    let (vmar_guard, old_vmar) = activate_vmar(ctx, new_vmar);
    apply_exec_cred(process, &ctx.credentials_mut(), exec_cred)?;
    drop(vmar_guard);
    drop(old_vmar);

    *thread_local.robust_list().borrow_mut() = None;
    thread_local.clear_child_tid().set(0);

    set_cpu_context(thread_local, user_context, elf_load_info);

    reset_vfork_child(process);

    unshare_and_close_files(ctx);

    *posix_thread.thread_name().lock() = thread_name;

    unshare_and_reset_sigdispositions(process);

    *thread_local.sig_stack().borrow_mut() = SigStack::default();

    process.set_exit_signal(SIGCHLD);

    Ok(())
}

fn wait_other_threads_exit(ctx: &Context) -> Result<()> {
    let is_main_thread = ctx.posix_thread.tid() == ctx.process.pid();

    let mut tasks = ctx.process.tasks().lock();
    loop {
        if is_main_thread {
            if tasks.as_slice().len() == 1 {
                return Ok(());
            }
        } else if tasks.as_slice().len() == 2 && tasks.has_exited_main() {
            return Ok(());
        }

        let (waiter, waker) = Waiter::new_pair();

        ctx.posix_thread
            .set_signalled_waker(waker.clone(), PauseReason::Sleep);
        if ctx.has_pending_sigkill() {
            ctx.posix_thread.clear_signalled_waker();
            return Err(errno!(EAGAIN, "the current thread has received SIGKILL"));
        }

        tasks.set_execve_waker(waker);
        drop(tasks);

        waiter.wait();

        ctx.posix_thread.clear_signalled_waker();

        tasks = ctx.process.tasks().lock();
        tasks.clear_execve_waker();
    }
}

fn make_current_main_thread(ctx: &Context) {
    let pid = ctx.process.pid();
    let old_tid = ctx.posix_thread.tid();

    if old_tid == pid {
        return;
    }

    let mut pid_table = pid_table::pid_table_mut();
    let tracer = ctx.posix_thread.tracer();
    let mut tracer_tracees = tracer
        .as_ref()
        .map(|tracer| tracer.as_posix_thread().unwrap().tracees().unwrap().lock());
    let mut tasks = ctx.process.tasks().lock();

    assert!(tasks.has_exited_main());
    assert!(tasks.in_execve());
    assert_eq!(tasks.as_slice().len(), 2);
    assert!(core::ptr::eq(ctx.task, tasks.as_slice()[1].as_ref()));

    tasks.swap_main();
    ctx.posix_thread.set_main(pid);

    if let Some(tracer_tracees) = tracer_tracees.as_mut()
        && let Some(tracee) = tracer_tracees.remove(&old_tid)
    {
        tracer_tracees.insert(pid, tracee);
    }

    drop(tasks);
    drop(tracer_tracees);

    let thread = pid_table.take_thread(old_tid).unwrap();
    pid_table.replace_thread(pid, &thread);
}

fn set_cpu_context(
    thread_local: &ThreadLocal,
    user_context: &mut UserContext,
    elf_load_info: &ElfLoadInfo,
) {
    let supp = thread_local.supp_user_context();

    supp.fpu().set(FpuContext::new());

    #[cfg(target_arch = "x86_64")]
    {
        supp.fs_base().set(FsBase::default());
        supp.gs_base().set(GsBase::default());
    }

    *user_context = UserContext::default();

    user_context.set_instruction_pointer(elf_load_info.entry_point as _);
    debug!("entry_point: 0x{:x}", elf_load_info.entry_point);

    user_context.set_stack_pointer(elf_load_info.user_stack_top as _);
    debug!("user stack top: 0x{:x}", elf_load_info.user_stack_top);
}

/// Applies prepared credentials.
fn apply_exec_cred(
    process: &Process,
    credentials: &Credentials<ReadWriteOp>,
    exec_cred: ExecCred,
) -> Result<()> {
    if exec_cred.will_change_ids() {
        process.clear_parent_death_signal();
    }
    credentials.apply_exec_cred(exec_cred)?;

    Ok(())
}

fn reset_vfork_child(process: &Process) {
    if process.status().is_vfork_child() {
        process.status().set_vfork_child(false);
        let parent = process.parent().lock().process().upgrade().unwrap();
        parent.children_wait_queue().wake_all();
    }
}

fn unshare_and_close_files(ctx: &Context) {
    ctx.unshare_files();

    let closed_files = {
        let file_table = ctx.thread_local.borrow_file_table();
        let mut file_table_locked = file_table.write();
        file_table_locked.close_files_on_exec()
    };
    drop(closed_files);
}

fn unshare_and_reset_sigdispositions(process: &Process) {
    let mut sig_dispositions = process.sig_dispositions().lock();

    let mut new = *sig_dispositions.lock();
    new.inherit();

    *sig_dispositions = Arc::new(Mutex::new(new));
}
