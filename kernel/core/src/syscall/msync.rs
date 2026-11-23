use align_ext::AlignExt;

use super::SyscallReturn;
use crate::{
    fs::file::SyncMode, prelude::*, thread::kernel_thread::ThreadOptions, vm::vmar::VMAR_CAP_ADDR,
};

pub(super) fn sys_msync(
    addr: Vaddr,
    len: usize,
    flag: i32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let flags = MsyncFlags::from_bits(flag).ok_or_else(|| errno!(EINVAL, "invalid flags"))?;
    debug!("addr = 0x{:x}, len = {}, flags = {:?}", addr, len, flags);

    if flags.contains(MsyncFlags::MS_ASYNC | MsyncFlags::MS_SYNC) {
        return Err(errno!(
            EINVAL,
            "MS_ASYNC and MS_SYNC cannot be specified together"
        ));
    }

    if !addr.is_multiple_of(PAGE_SIZE) {
        return Err(errno!(EINVAL, "the mapping address is not aligned"));
    }
    if len == 0 {
        return Ok(SyscallReturn::Return(0));
    }
    if VMAR_CAP_ADDR.checked_sub(addr).is_none_or(|gap| gap < len) {
        return Err(errno!(ENOMEM, "the mapping range is not in userspace"));
    }
    let addr_range = addr..(addr + len).align_up(PAGE_SIZE);

    let user_space = ctx.user_space();
    let vmar = user_space.vmar();
    let query_guard = vmar.query(addr_range.clone());

    if !query_guard.is_fully_mapped() {
        return Err(errno!(
            ENOMEM,
            "the range contains pages that are not mapped"
        ));
    }

    let inodes = query_guard
        .iter()
        .filter_map(|m| m.inode().cloned())
        .collect::<Vec<_>>();
    let task_fn = move || {
        for inode in inodes {
            let _ = inode.sync(SyncMode::Full);
        }
    };

    if flags.contains(MsyncFlags::MS_SYNC) {
        task_fn();
    } else {
        ThreadOptions::new(task_fn).spawn();
    }

    Ok(SyscallReturn::Return(0))
}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    struct MsyncFlags: i32 {
        const MS_ASYNC      = 0x01;
        const MS_INVALIDATE = 0x02;
        const MS_SYNC       = 0x04;
    }
}
