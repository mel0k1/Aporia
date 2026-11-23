use alloc::format;

use ostd::arch::cpu::context::UserContext;

use super::{SyscallReturn, constants::*};
use crate::{
    fs::{
        file::file_table::{FdFlags, FileDesc, RawFileDesc},
        vfs::path::{AT_FDCWD, EmptyPathStr, FsPath},
    },
    prelude::*,
    process::{
        ShebangScriptPath, UndetectedExecutable, do_execve,
        posix_thread::{ThreadName, derive_thread_name},
    },
};

pub(super) fn sys_execve(
    filename_ptr: Vaddr,
    argv_ptr_ptr: Vaddr,
    envp_ptr_ptr: Vaddr,
    ctx: &Context,
    user_context: &mut UserContext,
) -> Result<SyscallReturn> {
    let (executable, thread_name) = {
        let flags = OpenFlags::empty();
        lookup_and_open_executable_file(AT_FDCWD, filename_ptr, flags, ctx)?
    };

    do_execve(
        executable,
        thread_name,
        argv_ptr_ptr,
        envp_ptr_ptr,
        ctx,
        user_context,
    )?;
    Ok(SyscallReturn::NoReturn)
}

pub(super) fn sys_execveat(
    dfd: RawFileDesc,
    filename_ptr: Vaddr,
    argv_ptr_ptr: Vaddr,
    envp_ptr_ptr: Vaddr,
    flags: u32,
    ctx: &Context,
    user_context: &mut UserContext,
) -> Result<SyscallReturn> {
    let (executable, thread_name) = {
        let flags = OpenFlags::from_bits(flags).ok_or_else(|| errno!(EINVAL, "invalid flags"))?;
        lookup_and_open_executable_file(dfd, filename_ptr, flags, ctx)?
    };

    do_execve(
        executable,
        thread_name,
        argv_ptr_ptr,
        envp_ptr_ptr,
        ctx,
        user_context,
    )?;
    Ok(SyscallReturn::NoReturn)
}

fn lookup_and_open_executable_file(
    dfd: RawFileDesc,
    filename_ptr: Vaddr,
    flags: OpenFlags,
    ctx: &Context,
) -> Result<(UndetectedExecutable, ThreadName)> {
    let filename = ctx
        .user_space()
        .read_cstring(filename_ptr, MAX_FILENAME_LEN)?;

    let filename = filename.to_string_lossy();
    let path = {
        let fs_path = FsPath::from_fd_at(dfd, &filename, EmptyPathStr::AllowIfFlag(flags.bits()))?;

        let fs_ref = ctx.thread_local.borrow_fs();
        let path_resolver = fs_ref.resolver().read();
        if flags.contains(OpenFlags::AT_SYMLINK_NOFOLLOW) {
            path_resolver.lookup_no_follow(&fs_path)?
        } else {
            path_resolver.lookup(&fs_path)?
        }
    };

    let thread_name = if filename.is_empty() {
        derive_thread_name(&path.name())
    } else {
        derive_thread_name(&filename)
    };

    let shebang_script_path = if dfd == AT_FDCWD || filename.starts_with('/') {
        ShebangScriptPath::Accessible(CString::new(filename.into_owned()).unwrap())
    } else {
        let is_cloexec = if let Ok(fd) = FileDesc::try_from(dfd) {
            let file_table = ctx.thread_local.borrow_file_table();
            let file_table_locked = file_table.read();
            file_table_locked
                .get_entry(fd)
                .is_ok_and(|entry| entry.flags().contains(FdFlags::CLOEXEC))
        } else {
            false
        };

        if is_cloexec {
            ShebangScriptPath::Inaccessible
        } else {
            let path = if filename.is_empty() {
                format!("/dev/fd/{dfd}")
            } else {
                format!("/dev/fd/{dfd}/{filename}")
            };
            ShebangScriptPath::Accessible(CString::new(path).unwrap())
        }
    };

    let executable = UndetectedExecutable::open(path, shebang_script_path)?;

    Ok((executable, thread_name))
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug)]
    struct OpenFlags: u32 {
        const AT_EMPTY_PATH       = 0x1000;
        const AT_SYMLINK_NOFOLLOW = 0x100;
    }
}
