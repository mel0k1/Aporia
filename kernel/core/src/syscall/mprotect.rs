use align_ext::AlignExt;

use super::SyscallReturn;
use crate::{
    prelude::*,
    vm::{perms::VmPerms, vmar::VMAR_CAP_ADDR},
};

pub(super) fn sys_mprotect(
    addr: Vaddr,
    len: usize,
    perms: u64,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let vm_perms = VmPerms::from_user_bits(perms as u32)?;
    debug!(
        "addr = 0x{:x}, len = 0x{:x}, perms = {:?}",
        addr, len, vm_perms
    );

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

    #[cfg(any(target_arch = "x86_64", target_arch = "riscv64"))]
    let vm_perms = if !vm_perms.contains(VmPerms::READ) && vm_perms.contains(VmPerms::WRITE) {
        vm_perms | VmPerms::READ
    } else {
        vm_perms
    };

    let user_space = ctx.user_space();
    let vmar = user_space.vmar();
    vmar.protect(vm_perms, addr_range)?;

    Ok(SyscallReturn::Return(0))
}
