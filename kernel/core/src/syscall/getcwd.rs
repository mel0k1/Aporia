use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{fs::vfs::path::AbsPathResult, prelude::*};

pub(super) fn sys_getcwd(buf: Vaddr, len: usize, ctx: &Context) -> Result<SyscallReturn> {
    let abs_path = {
        let fs_ref = ctx.thread_local.borrow_fs();
        let path_resolver = fs_ref.resolver().read();
        path_resolver.make_abs_path(path_resolver.cwd())
    };

    debug!("getcwd: {:?}", abs_path);

    let abs_path = match abs_path {
        AbsPathResult::Reachable(s) => s,
        AbsPathResult::Unreachable(s) => alloc::format!("(unreachable){}", s),
    };

    let cwd = CString::new(abs_path)?;
    let bytes = cwd.as_bytes_with_nul();
    if bytes.len() > len {
        return Err(errno!(ERANGE, "the CWD buffer is too small"));
    }
    ctx.user_space().write_bytes(buf, bytes)?;

    Ok(SyscallReturn::Return(bytes.len() as _))
}
