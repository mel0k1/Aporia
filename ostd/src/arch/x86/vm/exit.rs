use int_to_c_enum::TryFromInt;
use x86::vmx::vmcs::ro;

use super::vmx::vmcs::CurrentVmcs;
use crate::{Error, prelude::*, vm::Gpaddr};

#[derive(Clone, Copy, Debug)]
pub struct GuestExitInfo {
    pub exit_reason: u32,
    pub instruction_len: u32,
    pub exit_qualification: u64,
    pub guest_phys_addr: Gpaddr,
    pub interruption_info: u32,
    pub interruption_error_code: u32,
    pub idt_vectoring_info: u32,
    pub idt_vectoring_error_code: u32,
}

#[expect(non_camel_case_types)]
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub enum VmxExitReason {
    EXCEPTION_NMI = 0,
    EXTERNAL_INTERRUPT = 1,
    TRIPLE_FAULT = 2,
    INIT = 3,
    SIPI = 4,
    SMI = 5,
    OTHER_SMI = 6,
    INTERRUPT_WINDOW = 7,
    NMI_WINDOW = 8,
    TASK_SWITCH = 9,
    CPUID = 10,
    GETSEC = 11,
    HLT = 12,
    INVD = 13,
    INVLPG = 14,
    RDPMC = 15,
    RDTSC = 16,
    RSM = 17,
    VMCALL = 18,
    VMCLEAR = 19,
    VMLAUNCH = 20,
    VMPTRLD = 21,
    VMPTRST = 22,
    VMREAD = 23,
    VMRESUME = 24,
    VMWRITE = 25,
    VMOFF = 26,
    VMON = 27,
    CR_ACCESS = 28,
    DR_ACCESS = 29,
    IO_INSTRUCTION = 30,
    MSR_READ = 31,
    MSR_WRITE = 32,
    INVALID_GUEST_STATE = 33,
    MSR_LOAD_FAIL = 34,
    MWAIT_INSTRUCTION = 36,
    MONITOR_TRAP_FLAG = 37,
    MONITOR_INSTRUCTION = 39,
    PAUSE_INSTRUCTION = 40,
    MCE_DURING_VMENTRY = 41,
    TPR_BELOW_THRESHOLD = 43,
    APIC_ACCESS = 44,
    VIRTUALIZED_EOI = 45,
    GDTR_IDTR = 46,
    LDTR_TR = 47,
    EPT_VIOLATION = 48,
    EPT_MISCONFIG = 49,
    INVEPT = 50,
    RDTSCP = 51,
    PREEMPTION_TIMER = 52,
    INVVPID = 53,
    WBINVD = 54,
    XSETBV = 55,
    APIC_WRITE = 56,
    RDRAND = 57,
    INVPCID = 58,
    VMFUNC = 59,
    ENCLS = 60,
    RDSEED = 61,
    PML_FULL = 62,
    XSAVES = 63,
    XRSTORS = 64,
    PCONFIG = 65,
    SPP_EVENT = 66,
    UMWAIT = 67,
    TPAUSE = 68,
    LOADIWKEY = 69,
}

pub(super) fn exit_info(vmcs: &CurrentVmcs<'_>) -> Result<GuestExitInfo> {
    let raw_reason = vmcs.read(ro::EXIT_REASON)?;
    if raw_reason & (1 << 31) != 0 {
        return Err(Error::InvalidArgs);
    }
    let reason = (raw_reason & 0xffff) as u32;

    let is_ept = reason == VmxExitReason::EPT_VIOLATION as u32
        || reason == VmxExitReason::EPT_MISCONFIG as u32;
    let guest_phys_addr = if is_ept {
        vmcs.read(ro::GUEST_PHYSICAL_ADDR_FULL)?
    } else {
        0
    };

    const ERROR_CODE_VALID: u32 = (1 << 31) | (1 << 11);

    let interruption_info = if reason == VmxExitReason::EXCEPTION_NMI as u32 {
        vmcs.read(ro::VMEXIT_INTERRUPTION_INFO)? as u32
    } else {
        0
    };
    let interruption_error_code = if interruption_info & ERROR_CODE_VALID == ERROR_CODE_VALID {
        vmcs.read(ro::VMEXIT_INTERRUPTION_ERR_CODE)? as u32
    } else {
        0
    };

    let idt_vectoring_info = vmcs.read(ro::IDT_VECTORING_INFO)? as u32;
    let idt_vectoring_error_code = if idt_vectoring_info & ERROR_CODE_VALID == ERROR_CODE_VALID {
        vmcs.read(ro::IDT_VECTORING_ERR_CODE)? as u32
    } else {
        0
    };

    Ok(GuestExitInfo {
        exit_reason: reason,
        instruction_len: vmcs.read(ro::VMEXIT_INSTRUCTION_LEN)? as u32,
        exit_qualification: vmcs.read(ro::EXIT_QUALIFICATION)? as u64,
        guest_phys_addr,
        interruption_info,
        interruption_error_code,
        idt_vectoring_info,
        idt_vectoring_error_code,
    })
}
