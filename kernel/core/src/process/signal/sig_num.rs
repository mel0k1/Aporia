#![expect(dead_code)]

use core::sync::atomic::{AtomicU8, Ordering};

use super::constants::*;
use crate::prelude::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SigNum {
    sig_num: u8,
}

impl TryFrom<u8> for SigNum {
    type Error = Error;

    fn try_from(sig_num: u8) -> Result<Self> {
        if !(MIN_STD_SIG_NUM..=MAX_RT_SIG_NUM).contains(&sig_num) {
            return Err(errno!(EINVAL, "invalid signal number"));
        }
        Ok(SigNum { sig_num })
    }
}

impl SigNum {
    pub(crate) const fn from_u8(sig_num: u8) -> Self {
        if sig_num > MAX_RT_SIG_NUM || sig_num < MIN_STD_SIG_NUM {
            panic!("invalid signal number")
        }
        SigNum { sig_num }
    }

    pub(crate) const fn as_u8(&self) -> u8 {
        self.sig_num
    }

    pub(crate) fn is_std(&self) -> bool {
        self.sig_num <= MAX_STD_SIG_NUM
    }

    pub(crate) fn is_real_time(&self) -> bool {
        self.sig_num >= MIN_RT_SIG_NUM
    }

    pub(crate) const fn sig_name(&self) -> &'static str {
        match *self {
            SIGHUP => "SIGHUP",
            SIGINT => "SIGINT",
            SIGQUIT => "SIGQUIT",
            SIGILL => "SIGILL",
            SIGTRAP => "SIGTRAP",
            SIGABRT => "SIGABRT",
            SIGBUS => "SIGBUS",
            SIGFPE => "SIGFPE",
            SIGKILL => "SIGKILL",
            SIGUSR1 => "SIGUSR1",
            SIGSEGV => "SIGSEGV",
            SIGUSR2 => "SIGUSR2",
            SIGPIPE => "SIGPIPE",
            SIGALRM => "SIGALRM",
            SIGTERM => "SIGTERM",
            SIGSTKFLT => "SIGSTKFLT",
            SIGCHLD => "SIGCHLD",
            SIGCONT => "SIGCONT",
            SIGSTOP => "SIGSTOP",
            SIGTSTP => "SIGTSTP",
            SIGTTIN => "SIGTTIN",
            SIGTTOU => "SIGTTOU",
            SIGURG => "SIGURG",
            SIGXCPU => "SIGXCPU",
            SIGXFSZ => "SIGXFSZ",
            SIGVTALRM => "SIGVTALRM",
            SIGPROF => "SIGPROF",
            SIGWINCH => "SIGWINCH",
            SIGIO => "SIGIO",
            SIGPWR => "SIGPWR",
            SIGSYS => "SIGSYS",
            _ => "Realtime Signal",
        }
    }
}

pub(crate) struct AtomicSigNum(AtomicU8);

impl AtomicSigNum {
    pub(crate) const fn new_empty() -> Self {
        Self(AtomicU8::new(0))
    }

    pub(crate) const fn new(sig_num: SigNum) -> Self {
        Self(AtomicU8::new(sig_num.as_u8()))
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.load(Ordering::Relaxed) == 0
    }

    pub(crate) fn as_sig_num(&self) -> Option<SigNum> {
        let sig_num = self.0.load(Ordering::Relaxed);
        if sig_num == 0 {
            return None;
        }

        Some(SigNum::from_u8(sig_num))
    }

    pub(crate) fn set(&self, sig_num: SigNum) {
        self.0.store(sig_num.as_u8(), Ordering::Relaxed)
    }

    pub(crate) fn clear(&self) {
        self.0.store(0, Ordering::Relaxed)
    }
}
