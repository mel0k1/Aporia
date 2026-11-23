pub(super) mod gdt;
pub(super) mod idt;
pub(super) mod syscall;

use super::cpu::context::GeneralRegs;
use crate::{
    arch::{
        cpu::context::CpuException,
        irq::{HwIrqLine, disable_local, enable_local},
    },
    cpu::PrivilegeLevel,
    irq::call_irq_callback_functions,
    mm::fault::TrapFrameApi,
};

cfg_select! {
    feature = "cvm_guest" => {
        use tdx_guest::{handle_virtual_exception, tdcall};

        use crate::arch::tdx_guest::TrapFrameWrapper;
    }
}

#[expect(missing_docs)]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct TrapFrame {
    pub rax: usize,
    pub rbx: usize,
    pub rcx: usize,
    pub rdx: usize,
    pub rsi: usize,
    pub rdi: usize,
    pub rbp: usize,
    pub r8: usize,
    pub r9: usize,
    pub r10: usize,
    pub r11: usize,
    pub r12: usize,
    pub r13: usize,
    pub r14: usize,
    pub r15: usize,

    pub trap_num: usize,
    pub error_code: usize,

    pub rip: usize,
    pub cs: usize,
    pub rflags: usize,
    pub rsp: usize,
    pub ss: usize,
}

crate::const_assert!(size_of::<TrapFrame>().is_multiple_of(16));

impl TrapFrameApi for TrapFrame {
    fn set_instruction_pointer(&mut self, ip: usize) {
        self.rip = ip;
    }

    fn instruction_pointer(&self) -> usize {
        self.rip
    }
}

pub(crate) unsafe fn init_on_cpu() {
    unsafe { gdt::init_on_cpu() };

    idt::init_on_cpu();

    unsafe { syscall::init_on_cpu() };
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct RawUserContext {
    pub(super) general: GeneralRegs,
    pub(super) rflags: usize,
    pub(super) trap_num: usize,
    pub(super) error_code: usize,
}

#[unsafe(no_mangle)]
unsafe extern "sysv64" fn trap_handler(f: &mut TrapFrame) {
    fn enable_local_if(cond: bool) {
        if cond {
            enable_local();
        }
    }

    fn disable_local_if(cond: bool) {
        if cond {
            disable_local();
        }
    }

    let was_irq_enabled =
        f.rflags as u64 & x86_64::registers::rflags::RFlags::INTERRUPT_FLAG.bits() > 0;

    let cpu_exception = CpuException::new(f.trap_num, f.error_code);
    match cpu_exception {
        #[cfg(feature = "cvm_guest")]
        Some(CpuException::VirtualizationException) => {
            let ve_info = tdcall::get_veinfo().expect("#VE handler: fail to get VE info\n");
            enable_local_if(was_irq_enabled);
            let mut trapframe_wrapper = TrapFrameWrapper(&mut *f);
            handle_virtual_exception(&mut trapframe_wrapper, &ve_info);
            *f = *trapframe_wrapper.0;
            disable_local_if(was_irq_enabled);
        }
        Some(page_fault @ CpuException::PageFault(raw_page_fault_info)) => {
            enable_local_if(was_irq_enabled);
            crate::mm::fault::handle_user_page_fault(f, &page_fault, raw_page_fault_info.addr);
            disable_local_if(was_irq_enabled);
        }
        Some(exception) => {
            enable_local_if(was_irq_enabled);
            panic!(
                "Cannot handle kernel CPU exception: {:#x?}; trapframe: {:#x?}",
                exception, f
            );
        }
        None => {
            call_irq_callback_functions(
                f,
                &HwIrqLine::new(f.trap_num as u8),
                PrivilegeLevel::Kernel,
            );
        }
    }
}

pub const USER_CS_VALUE: usize = gdt::USER_CS.0 as usize;
pub const USER_SS_VALUE: usize = gdt::USER_SS.0 as usize;
