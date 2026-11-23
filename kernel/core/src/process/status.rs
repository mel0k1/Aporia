use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use ostd::sync::SpinLock;

use super::ExitCode;
use crate::process::{WaitOptions, signal::sig_num::SigNum};

#[derive(Debug)]
pub(crate) struct ProcessStatus {
    is_zombie: AtomicBool,
    is_vfork_child: AtomicBool,
    exit_code: AtomicU32,
    stop_status: StopStatus,
}

impl Default for ProcessStatus {
    fn default() -> Self {
        Self {
            is_zombie: AtomicBool::new(false),
            is_vfork_child: AtomicBool::new(false),
            exit_code: AtomicU32::new(0),
            stop_status: StopStatus::new(),
        }
    }
}

impl ProcessStatus {
    pub(crate) fn is_zombie(&self) -> bool {
        self.is_zombie.load(Ordering::Acquire)
    }

    pub(super) fn set_zombie(&self) {
        self.is_zombie.store(true, Ordering::Release);
    }
}

impl ProcessStatus {
    pub(crate) fn is_vfork_child(&self) -> bool {
        self.is_vfork_child.load(Ordering::Acquire)
    }

    pub(crate) fn set_vfork_child(&self, is_vfork_child: bool) {
        self.is_vfork_child.store(is_vfork_child, Ordering::Release);
    }
}

impl ProcessStatus {
    pub(crate) fn exit_code(&self) -> ExitCode {
        self.exit_code.load(Ordering::Relaxed)
    }

    pub(super) fn set_exit_code(&self, exit_code: ExitCode) {
        self.exit_code.store(exit_code, Ordering::Relaxed);
    }
}

impl ProcessStatus {
    pub(super) fn stop_status(&self) -> &StopStatus {
        &self.stop_status
    }
}

#[derive(Debug)]
pub(super) struct StopStatus {
    is_stopped: AtomicBool,

    wait_status: SpinLock<Option<StopWaitStatus>>,
}

impl StopStatus {
    pub(self) const fn new() -> Self {
        Self {
            is_stopped: AtomicBool::new(false),
            wait_status: SpinLock::new(None),
        }
    }

    pub(super) fn stop(&self, signum: SigNum) -> bool {
        let mut wait_status = self.wait_status.lock();

        if self.is_stopped.load(Ordering::Relaxed) {
            false
        } else {
            self.is_stopped.store(true, Ordering::Relaxed);
            *wait_status = Some(StopWaitStatus::Stopped(signum));
            true
        }
    }

    pub(super) fn resume(&self) -> bool {
        let mut wait_status = self.wait_status.lock();

        if self.is_stopped.load(Ordering::Relaxed) {
            self.is_stopped.store(false, Ordering::Relaxed);
            *wait_status = Some(StopWaitStatus::Continue);
            true
        } else {
            false
        }
    }

    pub(super) fn is_stopped(&self) -> bool {
        self.is_stopped.load(Ordering::Relaxed)
    }

    pub(super) fn wait(&self, options: WaitOptions) -> Option<StopWaitStatus> {
        let mut wait_status = self.wait_status.lock();

        if options.contains(WaitOptions::WSTOPPED)
            && let Some(StopWaitStatus::Stopped(_)) = wait_status.as_ref()
        {
            return if options.contains(WaitOptions::WNOWAIT) {
                wait_status.as_ref().cloned()
            } else {
                wait_status.take()
            };
        }

        if options.contains(WaitOptions::WCONTINUED)
            && let Some(StopWaitStatus::Continue) = wait_status.as_ref()
        {
            return if options.contains(WaitOptions::WNOWAIT) {
                wait_status.as_ref().cloned()
            } else {
                wait_status.take()
            };
        }

        None
    }
}

#[derive(Clone, Debug)]
pub(super) enum StopWaitStatus {
    Stopped(SigNum),
    Continue,
}
