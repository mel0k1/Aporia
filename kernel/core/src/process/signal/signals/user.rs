#![expect(dead_code)]

use ostd::mm::Vaddr;

use super::Signal;
use crate::{
    context::Context,
    process::{
        Pid, Uid,
        signal::{
            c_types::siginfo_t,
            constants::{SI_QUEUE, SI_TKILL, SI_USER},
            sig_num::SigNum,
        },
    },
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct UserSignal {
    num: SigNum,
    pid: Pid,
    uid: Uid,
    kind: UserSignalKind,
    sigval: Vaddr,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum UserSignalKind {
    Kill,
    Tkill,
    Sigqueue,
}

impl UserSignal {
    pub(crate) fn new(num: SigNum, kind: UserSignalKind, pid: Pid, uid: Uid) -> Self {
        Self {
            num,
            pid,
            uid,
            kind,
            sigval: 0,
        }
    }

    pub(crate) fn new_kill(num: SigNum, ctx: &Context) -> Self {
        Self {
            num,
            pid: ctx.process.pid(),
            uid: ctx.posix_thread.credentials().ruid(),
            kind: UserSignalKind::Kill,
            sigval: 0,
        }
    }

    pub(crate) fn new_sigqueue(num: SigNum, ctx: &Context, sigval: Vaddr) -> Self {
        Self {
            num,
            pid: ctx.process.pid(),
            uid: ctx.posix_thread.credentials().ruid(),
            kind: UserSignalKind::Sigqueue,
            sigval,
        }
    }

    pub(crate) fn new_tkill_with_sigval(num: SigNum, ctx: &Context, sigval: Vaddr) -> Self {
        Self {
            num,
            pid: ctx.process.pid(),
            uid: ctx.posix_thread.credentials().ruid(),
            kind: UserSignalKind::Tkill,
            sigval,
        }
    }

    pub(crate) fn pid(&self) -> Pid {
        self.pid
    }

    pub(crate) fn kind(&self) -> UserSignalKind {
        self.kind
    }
}

impl Signal for UserSignal {
    fn num(&self) -> SigNum {
        self.num
    }

    fn to_info(&self) -> siginfo_t {
        let code = match self.kind {
            UserSignalKind::Kill => SI_USER,
            UserSignalKind::Tkill => SI_TKILL,
            UserSignalKind::Sigqueue => SI_QUEUE,
        };

        let mut info = siginfo_t::new(self.num, code);
        info.set_pid_uid(self.pid, self.uid);
        info.set_ptr(self.sigval);

        info
    }
}
