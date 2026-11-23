use loongArch64::register::estat::Exception;
use ostd::{
    arch::cpu::context::{CpuExceptionInfo, UserContext},
    user::UserContextApi,
};

use crate::{
    process::signal::{SignalContext, sig_num::SigNum, signals::fault::FaultSignal},
    thread::exception::ToFaultSignal,
};

impl SignalContext for UserContext {
    fn set_arguments(&mut self, sig_num: SigNum, siginfo_addr: usize, ucontext_addr: usize) {
        self.set_a0(sig_num.as_u8() as usize);
        self.set_a1(siginfo_addr);
        self.set_a2(ucontext_addr);
    }
}

impl ToFaultSignal for CpuExceptionInfo {
    fn to_fault_signal(&self, user_ctx: &UserContext) -> Option<FaultSignal> {
        use crate::process::signal::constants::*;

        let era = user_ctx.instruction_pointer() as u64;

        let (num, code, addr) = match self.code {
            Exception::LoadPageFault | Exception::StorePageFault | Exception::FetchPageFault => {
                (SIGSEGV, SEGV_MAPERR, Some(self.page_fault_addr as u64))
            }
            Exception::PageModifyFault
            | Exception::PageNonReadableFault
            | Exception::PageNonExecutableFault
            | Exception::PagePrivilegeIllegal => {
                (SIGSEGV, SEGV_ACCERR, Some(self.page_fault_addr as u64))
            }
            Exception::FetchInstructionAddressError | Exception::MemoryAccessAddressError => {
                (SIGBUS, BUS_ADRERR, None)
            }
            Exception::AddressNotAligned => (SIGBUS, BUS_ADRALN, None),
            Exception::BoundsCheckFault => (SIGSEGV, SEGV_BNDERR, None),
            Exception::Breakpoint => (SIGTRAP, TRAP_BRKPT, Some(era)),
            Exception::InstructionNotExist | Exception::InstructionPrivilegeIllegal => {
                (SIGILL, SI_KERNEL, None)
            }
            Exception::FloatingPointUnavailable => (SIGFPE, FPE_FLTINV, Some(era)),

            Exception::Syscall | Exception::TLBRFill => return None,
        };

        Some(FaultSignal::new(num, code, addr))
    }
}
