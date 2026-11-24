use align_ext::AlignExt;

use super::SyscallReturn;
use crate::{prelude::*, vm::vmar::VMAR_CAP_ADDR};

/// The flag of `mlock2` that locks pages in the "lock on fault" mode.
const MLOCK_ONFAULT: u32 = 0x01;
/// The flag of `mlockall` that locks all pages that are currently mapped.
const MCL_CURRENT: u32 = 0x01;
/// The flag of `mlockall` that locks all pages that will be mapped in the future.
const MCL_FUTURE: u32 = 0x02;
/// The flag of `mlockall` that locks pages in the "lock on fault" mode.
const MCL_ONFAULT: u32 = 0x04;

pub(super) fn sys_mlock(addr: Vaddr, len: usize, ctx: &Context) -> Result<SyscallReturn> {
    debug!("addr = 0x{:x}, len = 0x{:x}", addr, len);
    do_mlock(addr, len, false, ctx)
}

pub(super) fn sys_mlock2(
    addr: Vaddr,
    len: usize,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "addr = 0x{:x}, len = 0x{:x}, flags = 0x{:x}",
        addr, len, flags
    );
    if flags & !MLOCK_ONFAULT != 0 {
        return Err(errno!(EINVAL, "invalid mlock2 flags"));
    }
    do_mlock(addr, len, flags == MLOCK_ONFAULT, ctx)
}

fn do_mlock(addr: Vaddr, len: usize, lock_onfault: bool, ctx: &Context) -> Result<SyscallReturn> {
    if !addr.is_multiple_of(PAGE_SIZE) {
        return Err(errno!(EINVAL, "the mapping address is not aligned"));
    }
    if len == 0 {
        return Ok(SyscallReturn::Return(0));
    }
    if VMAR_CAP_ADDR.checked_sub(addr).is_none_or(|gap| gap < len) {
        return Err(errno!(EINVAL, "the mapping range is not in userspace"));
    }
    let addr_range = addr..(addr + len).align_up(PAGE_SIZE);

    ctx.user_space().vmar().lock(addr_range, lock_onfault)?;

    Ok(SyscallReturn::Return(0))
}

pub(super) fn sys_munlock(addr: Vaddr, len: usize, ctx: &Context) -> Result<SyscallReturn> {
    debug!("addr = 0x{:x}, len = 0x{:x}", addr, len);

    if !addr.is_multiple_of(PAGE_SIZE) {
        return Err(errno!(EINVAL, "the mapping address is not aligned"));
    }
    if len == 0 {
        return Ok(SyscallReturn::Return(0));
    }
    if VMAR_CAP_ADDR.checked_sub(addr).is_none_or(|gap| gap < len) {
        return Err(errno!(EINVAL, "the mapping range is not in userspace"));
    }
    let addr_range = addr..(addr + len).align_up(PAGE_SIZE);

    ctx.user_space().vmar().unlock(addr_range)?;

    Ok(SyscallReturn::Return(0))
}

pub(super) fn sys_mlockall(flags: u32, ctx: &Context) -> Result<SyscallReturn> {
    debug!("flags = 0x{:x}", flags);

    if flags & !(MCL_CURRENT | MCL_FUTURE | MCL_ONFAULT) != 0 || flags == 0 {
        return Err(errno!(EINVAL, "invalid mlockall flags"));
    }
    if flags & MCL_ONFAULT != 0 && flags & (MCL_CURRENT | MCL_FUTURE) == 0 {
        return Err(errno!(
            EINVAL,
            "MCL_ONFAULT must be combined with MCL_CURRENT or MCL_FUTURE"
        ));
    }

    let user_space = ctx.user_space();
    let vmar = user_space.vmar();

    if flags & MCL_FUTURE != 0 {
        vmar.set_lock_future(true, flags & MCL_ONFAULT != 0);
    }
    if flags & MCL_CURRENT != 0 {
        vmar.lock_all(flags & MCL_ONFAULT != 0)?;
    }

    Ok(SyscallReturn::Return(0))
}

pub(super) fn sys_munlockall(ctx: &Context) -> Result<SyscallReturn> {
    debug!("munlockall");

    let user_space = ctx.user_space();
    user_space.vmar().unlock_all()?;

    Ok(SyscallReturn::Return(0))
}
