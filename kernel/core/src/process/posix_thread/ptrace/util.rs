use crate::{
    prelude::*,
    process::{
        ExitCode, WaitOptions,
        signal::{
            DequeuedSignal, c_types::siginfo_t, constants::SIGTRAP, sig_num::SigNum,
            signals::Signal,
        },
    },
    thread::Tid,
};

pub(crate) enum PtraceContRequest {
    Continue(Option<SigNum>),
    #[cfg_attr(not(target_arch = "x86_64"), expect(dead_code))]
    SingleStep(Option<SigNum>),
    Syscall(Option<SigNum>),
}

impl PtraceContRequest {
    pub(super) fn sig_num(&self) -> Option<SigNum> {
        match self {
            Self::Continue(Some(sig_num))
            | Self::SingleStep(Some(sig_num))
            | Self::Syscall(Some(sig_num)) => Some(*sig_num),
            _ => None,
        }
    }
}

pub(crate) enum PtraceStopResult {
    Continued(Option<DequeuedSignal>),
    Interrupted,
    NotTraced(Option<DequeuedSignal>),
}

#[derive(Default)]
pub(super) enum StopDeliverySignal {
    Pending(DequeuedSignal, PtraceWaitStatus),
    Consumed(DequeuedSignal),
    Injected(DequeuedSignal),
    #[default]
    Empty,
}

impl StopDeliverySignal {
    pub(super) fn stop(&mut self, signal: DequeuedSignal, wait_status: PtraceWaitStatus) {
        *self = Self::Pending(signal, wait_status);
    }

    pub(super) fn clear(&mut self) -> Option<DequeuedSignal> {
        let this = core::mem::replace(self, Self::Empty);

        match this {
            Self::Pending(signal, _) | Self::Injected(signal) => Some(signal),
            Self::Consumed(_) | Self::Empty => None,
        }
    }

    pub(super) fn wait(&mut self, options: WaitOptions) -> Option<PtraceWaitStatus> {
        let this = core::mem::replace(self, Self::Empty);

        match this {
            Self::Pending(signal, wait_status) => {
                if !options.contains(WaitOptions::WNOWAIT) {
                    *self = Self::Consumed(signal);
                } else {
                    *self = Self::Pending(signal, wait_status);
                }
                Some(wait_status)
            }
            Self::Consumed(signal) => {
                *self = Self::Consumed(signal);
                None
            }
            Self::Injected(_) => unreachable!(),
            Self::Empty => None,
        }
    }

    pub(super) fn inject(&mut self, new_signal: Box<dyn Signal>) {
        let this = core::mem::replace(self, Self::Empty);

        let mut signal = match this {
            Self::Pending(signal, _) | Self::Consumed(signal) => signal,
            Self::Injected(_) | Self::Empty => unreachable!(),
        };

        signal.set_signal(new_signal);
        *self = Self::Injected(signal);
    }

    pub(super) fn get(&self) -> Option<&dyn Signal> {
        match self {
            Self::Pending(signal, _) | Self::Consumed(signal) | Self::Injected(signal) => {
                Some(signal.signal())
            }
            Self::Empty => None,
        }
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub(crate) struct PtraceOptions: usize {
        const PTRACE_O_TRACESYSGOOD   = 1 << 0;
        const PTRACE_O_TRACEFORK      = 1 << PtraceEvent::Fork(0).code();
        const PTRACE_O_TRACEVFORK     = 1 << PtraceEvent::Vfork(0).code();
        const PTRACE_O_TRACECLONE     = 1 << PtraceEvent::Clone(0).code();
        const PTRACE_O_TRACEEXEC      = 1 << PtraceEvent::Exec(0).code();
        const PTRACE_O_TRACEVFORKDONE = 1 << PtraceEvent::VforkDone(0).code();
        const PTRACE_O_TRACEEXIT      = 1 << PtraceEvent::Exit(0).code();
        const PTRACE_O_EXITKILL       = 1 << 20;
    }
}

#[derive(Debug, Clone)]
pub(crate) enum PtraceEvent {
    Fork(Tid),
    Vfork(Tid),
    Clone(Tid),
    Exec(Tid),
    VforkDone(Tid),
    Exit(ExitCode),
}

impl PtraceEvent {
    const fn code(&self) -> u32 {
        match self {
            Self::Fork(_) => 1,
            Self::Vfork(_) => 2,
            Self::Clone(_) => 3,
            Self::Exec(_) => 4,
            Self::VforkDone(_) => 5,
            Self::Exit(_) => 6,
        }
    }

    pub(super) const fn option(&self) -> PtraceOptions {
        PtraceOptions::from_bits(1 << self.code()).unwrap()
    }

    pub(crate) const fn message(&self) -> usize {
        match self {
            Self::Fork(tid)
            | Self::Vfork(tid)
            | Self::Clone(tid)
            | Self::Exec(tid)
            | Self::VforkDone(tid) => *tid as usize,
            Self::Exit(exit_code) => *exit_code as usize,
        }
    }

    pub(super) fn siginfo(&self, ctx: &Context) -> siginfo_t {
        let code = PtraceWaitStatus::from_event(self).0;
        let mut siginfo = siginfo_t::new(SIGTRAP, code);
        siginfo.set_pid_uid_by(ctx);
        siginfo
    }
}

#[derive(Copy, Clone)]
pub(crate) struct PtraceWaitStatus(i32);

impl PtraceWaitStatus {
    pub(super) fn from_event(event: &PtraceEvent) -> Self {
        Self(SIGTRAP.as_u8() as i32 | ((event.code() as i32) << 8))
    }

    pub(super) fn from_signal(sig: SigNum) -> Self {
        Self(sig.as_u8() as i32)
    }

    pub(super) fn from_syscall(options: &PtraceOptions) -> Self {
        let mut sig = SIGTRAP.as_u8() as i32;
        if options.contains(PtraceOptions::PTRACE_O_TRACESYSGOOD) {
            sig |= 0x80;
        }
        Self(sig)
    }

    pub(crate) fn to_wait4_status(self) -> u32 {
        ((self.0 as u32) << 8) | 0x7f
    }

    pub(crate) fn to_waitid_si_status(self) -> i32 {
        self.0
    }
}

pub(super) fn syscall_stop_siginfo(options: &PtraceOptions, ctx: &Context) -> siginfo_t {
    let code = PtraceWaitStatus::from_syscall(options).0;
    let mut siginfo = siginfo_t::new(SIGTRAP, code);
    siginfo.set_pid_uid_by(ctx);
    siginfo
}
