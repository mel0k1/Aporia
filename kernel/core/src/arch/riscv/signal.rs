use ostd::{
    arch::cpu::context::{CpuException, UserContext},
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

impl ToFaultSignal for CpuException {
    fn to_fault_signal(&self, user_ctx: &UserContext) -> Option<FaultSignal> {
        use CpuException::*;

        use crate::process::signal::constants::*;

        let sepc = user_ctx.instruction_pointer() as u64;

        let (num, code, addr) = match self {
            InstructionMisaligned => (SIGBUS, BUS_ADRALN, sepc),
            InstructionFault => (SIGSEGV, SEGV_ACCERR, sepc),
            IllegalInstruction(_) => (SIGILL, ILL_ILLOPC, sepc),
            Breakpoint => (SIGTRAP, TRAP_BRKPT, sepc),
            LoadMisaligned(_) | StoreMisaligned(_) => (SIGBUS, BUS_ADRALN, sepc),
            LoadFault(_) | StoreFault(_) => (SIGSEGV, SEGV_ACCERR, sepc),
            SupervisorEnvCall => (SIGILL, ILL_ILLTRP, sepc),
            InstructionPageFault(addr) | LoadPageFault(addr) | StorePageFault(addr) => {
                (SIGSEGV, SEGV_MAPERR, *addr as u64)
            }
            Unknown => (SIGILL, ILL_ILLTRP, sepc),

            UserEnvCall => return None,
        };

        Some(FaultSignal::new(num, code, Some(addr)))
    }
}
