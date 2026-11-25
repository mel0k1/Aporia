use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::prelude::*;

pub(super) fn sys_mincore(
    addr: Vaddr,
    len: usize,
    vec_ptr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "addr = 0x{:x}, len = 0x{:x}, vec = 0x{:x}",
        addr, len, vec_ptr
    );

    if !addr.is_multiple_of(PAGE_SIZE) {
        return Err(errno!(EINVAL, "the address is not page-aligned"));
    }

    if addr.checked_add(len).is_none() {
        return Err(errno!(EINVAL, "the address range overflows"));
    }

    if len == 0 {
        return Ok(SyscallReturn::Return(0));
    }

    let mut bits = vec![0u8; len.div_ceil(PAGE_SIZE)];

    {
        let vmar_guard = ctx.process.lock_vmar();
        let vmar = vmar_guard
            .as_ref()
            .ok_or_else(|| errno!(ENOMEM, "the process has no address space"))?;
        vmar.mincore_range(addr..addr + len, &mut bits)?;
    }

    ctx.user_space()
        .write(vec_ptr, &mut VmReader::from(bits.as_slice()).to_fallible())?;

    Ok(SyscallReturn::Return(0))
}
