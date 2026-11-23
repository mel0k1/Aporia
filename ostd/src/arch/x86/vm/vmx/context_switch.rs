use super::super::types::VcpuRegs;
use crate::irq::DisabledLocalIrqGuard;

pub(super) unsafe fn vcpu_run(
    regs: &mut VcpuRegs,
    is_launched: bool,
    _irq_guard: &DisabledLocalIrqGuard,
) -> u64 {
    unsafe { __vcpu_run(regs, u64::from(is_launched)) }
}

pub(in crate::arch::vm) fn vm_exit_handler_virtaddr() -> usize {
    __vm_exit_handler as *const () as usize
}

unsafe extern "C" {
    fn __vcpu_run(regs: *mut VcpuRegs, launched: u64) -> u64;
    fn __vm_exit_handler();
}

core::arch::global_asm!(
    include_str!("context_switch.S"),
    host_rsp = const x86::vmx::vmcs::host::RSP,
    rax = const core::mem::offset_of!(VcpuRegs, rax),
    rbx = const core::mem::offset_of!(VcpuRegs, rbx),
    rcx = const core::mem::offset_of!(VcpuRegs, rcx),
    rdx = const core::mem::offset_of!(VcpuRegs, rdx),
    rsi = const core::mem::offset_of!(VcpuRegs, rsi),
    rdi = const core::mem::offset_of!(VcpuRegs, rdi),
    rbp = const core::mem::offset_of!(VcpuRegs, rbp),
    r8 = const core::mem::offset_of!(VcpuRegs, r8),
    r9 = const core::mem::offset_of!(VcpuRegs, r9),
    r10 = const core::mem::offset_of!(VcpuRegs, r10),
    r11 = const core::mem::offset_of!(VcpuRegs, r11),
    r12 = const core::mem::offset_of!(VcpuRegs, r12),
    r13 = const core::mem::offset_of!(VcpuRegs, r13),
    r14 = const core::mem::offset_of!(VcpuRegs, r14),
    r15 = const core::mem::offset_of!(VcpuRegs, r15),
);
