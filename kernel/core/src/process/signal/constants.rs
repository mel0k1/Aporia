#![expect(dead_code)]

pub(super) const MIN_STD_SIG_NUM: u8 = 1;
pub(super) const MAX_STD_SIG_NUM: u8 = 31;
pub(super) const MIN_RT_SIG_NUM: u8 = 32;
pub(super) const MAX_RT_SIG_NUM: u8 = 64;
pub(super) const COUNT_STD_SIGS: usize = 31;
pub(super) const COUNT_RT_SIGS: usize = 33;
pub(super) const COUNT_ALL_SIGS: usize = 64;

pub(crate) const SIG_DFL: usize = 0;
pub(crate) const SIG_IGN: usize = 1;

use super::sig_num::SigNum;

macro_rules! define_std_signums {
    ( $( $name: ident = $num: expr ),+, ) => {
        $(
            pub(crate) const $name : SigNum = SigNum::from_u8($num);
        )*
    }
}

define_std_signums! {
    SIGHUP    = 1,
    SIGINT    = 2,
    SIGQUIT   = 3,
    SIGILL    = 4,
    SIGTRAP   = 5,
    SIGABRT   = 6,
    SIGBUS    = 7,
    SIGFPE    = 8,
    SIGKILL   = 9,
    SIGUSR1   = 10,
    SIGSEGV   = 11,
    SIGUSR2   = 12,
    SIGPIPE   = 13,
    SIGALRM   = 14,
    SIGTERM   = 15,
    SIGSTKFLT = 16,
    SIGCHLD   = 17,
    SIGCONT   = 18,
    SIGSTOP   = 19,
    SIGTSTP   = 20,
    SIGTTIN   = 21,
    SIGTTOU   = 22,
    SIGURG    = 23,
    SIGXCPU   = 24,
    SIGXFSZ   = 25,
    SIGVTALRM = 26,
    SIGPROF   = 27,
    SIGWINCH  = 28,
    SIGIO     = 29,
    SIGPWR    = 30,
    SIGSYS    = 31,
}

pub(crate) const SI_ASYNCNL: i32 = -60;
pub(crate) const SI_TKILL: i32 = -6;
pub(crate) const SI_SIGIO: i32 = -5;
pub(crate) const SI_ASYNCIO: i32 = -4;
pub(crate) const SI_MESGQ: i32 = -3;
pub(crate) const SI_TIMER: i32 = -2;
pub(crate) const SI_QUEUE: i32 = -1;
pub(crate) const SI_USER: i32 = 0;
pub(crate) const SI_KERNEL: i32 = 128;

pub(crate) const FPE_INTDIV: i32 = 1;
pub(crate) const FPE_INTOVF: i32 = 2;
pub(crate) const FPE_FLTDIV: i32 = 3;
pub(crate) const FPE_FLTOVF: i32 = 4;
pub(crate) const FPE_FLTUND: i32 = 5;
pub(crate) const FPE_FLTRES: i32 = 6;
pub(crate) const FPE_FLTINV: i32 = 7;
pub(crate) const FPE_FLTSUB: i32 = 8;

pub(crate) const ILL_ILLOPC: i32 = 1;
pub(crate) const ILL_ILLOPN: i32 = 2;
pub(crate) const ILL_ILLADR: i32 = 3;
pub(crate) const ILL_ILLTRP: i32 = 4;
pub(crate) const ILL_PRVOPC: i32 = 5;
pub(crate) const ILL_PRVREG: i32 = 6;
pub(crate) const ILL_COPROC: i32 = 7;
pub(crate) const ILL_BADSTK: i32 = 8;

pub(crate) const SEGV_MAPERR: i32 = 1;
pub(crate) const SEGV_ACCERR: i32 = 2;
pub(crate) const SEGV_BNDERR: i32 = 3;
pub(crate) const SEGV_PKUERR: i32 = 4;

pub(crate) const BUS_ADRALN: i32 = 1;
pub(crate) const BUS_ADRERR: i32 = 2;
pub(crate) const BUS_OBJERR: i32 = 3;
pub(crate) const BUS_MCEERR_AR: i32 = 4;
pub(crate) const BUS_MCEERR_AO: i32 = 5;

pub(crate) const CLD_EXITED: i32 = 1;
pub(crate) const CLD_KILLED: i32 = 2;
pub(crate) const CLD_DUMPED: i32 = 3;
pub(crate) const CLD_TRAPPED: i32 = 4;
pub(crate) const CLD_STOPPED: i32 = 5;
pub(crate) const CLD_CONTINUED: i32 = 6;

pub(crate) const TRAP_BRKPT: i32 = 1;
pub(crate) const TRAP_TRACE: i32 = 2;
pub(crate) const TRAP_BRANCH: i32 = 3;
pub(crate) const TRAP_HWBKPT: i32 = 4;
pub(crate) const TRAP_UNK: i32 = 5;
pub(crate) const TRAP_PERF: i32 = 6;
