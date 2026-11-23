use ostd::{cpu::CpuId, mm::VmIo};

use super::SyscallReturn;
use crate::prelude::*;

pub(super) fn sys_getcpu(
    cpu: Vaddr,
    node: Vaddr,
    _tcache: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let current_cpu: u32 = CpuId::current_racy().into();
    let current_node = 0u32;

    debug!(
        "[sys_getcpu]: cpu = {} (total {}), node = {}",
        current_cpu,
        ostd::cpu::num_cpus(),
        current_node,
    );

    if cpu != 0 {
        ctx.user_space().write_val(cpu, &current_cpu)?;
    }
    if node != 0 {
        ctx.user_space().write_val(node, &current_node)?;
    }

    Ok(SyscallReturn::Return(0))
}
