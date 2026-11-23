use ostd::{
    arch::cpu::context::{CpuException, PageFaultErrorCode, UserContext},
    user::UserContextApi,
};

use crate::{
    process::signal::{SignalContext, sig_num::SigNum, signals::fault::FaultSignal},
    thread::exception::ToFaultSignal,
};

impl SignalContext for UserContext {
    fn set_arguments(&mut self, sig_num: SigNum, siginfo_addr: usize, ucontext_addr: usize) {
        self.set_rdi(sig_num.as_u8() as usize);
        self.set_rsi(siginfo_addr);
        self.set_rdx(ucontext_addr);
    }
}

impl ToFaultSignal for CpuException {
    fn to_fault_signal(&self, user_ctx: &UserContext) -> Option<FaultSignal> {
        use crate::process::signal::constants::*;

        let rip = user_ctx.instruction_pointer() as u64;

        let (num, code, addr) = match self {
            CpuException::DivisionError => (SIGFPE, FPE_INTDIV, Some(rip)),
            CpuException::Debug => (SIGTRAP, TRAP_TRACE, Some(rip)),
            CpuException::BreakPoint => (SIGTRAP, SI_KERNEL, None),
            CpuException::Overflow => (SIGSEGV, SI_KERNEL, None),
            CpuException::InvalidOpcode => (SIGILL, ILL_ILLOPN, Some(rip)),
            CpuException::StackSegmentFault(..) => (SIGBUS, SI_KERNEL, None),
            CpuException::GeneralProtectionFault(..) => (SIGSEGV, SI_KERNEL, None),
            CpuException::PageFault(raw_page_fault_info) => {
                let code = if raw_page_fault_info
                    .error_code
                    .contains(PageFaultErrorCode::PRESENT)
                {
                    SEGV_ACCERR
                } else {
                    SEGV_MAPERR
                };
                let addr = Some(raw_page_fault_info.addr as u64);
                (SIGSEGV, code, addr)
            }
            CpuException::X87FloatingPointException | CpuException::SIMDFloatingPointException => {
                (SIGFPE, FPE_FLTDIV, Some(rip))
            }
            CpuException::AlignmentCheck => (SIGBUS, BUS_ADRALN, None),

            _ => return None,
        };

        Some(FaultSignal::new(num, code, addr))
    }
}
