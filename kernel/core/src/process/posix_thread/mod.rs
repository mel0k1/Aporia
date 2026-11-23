use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use aster_rights::{ReadDupOp, ReadOp, ReadWriteOp};
use aster_util::fixed_str::FixedCStr;
use ostd::{
    sync::{RoArc, RwMutexReadGuard, Waker},
    task::Task,
};
use spin::Once;

use super::{
    Credentials, Process,
    signal::{sig_mask::AtomicSigMask, sig_num::SigNum, sig_queues::SigQueues, signals::Signal},
};
use crate::{
    events::IoEvents,
    fs::{file::file_table::FileTable, thread_info::ThreadFsInfo},
    prelude::*,
    process::{
        ExitCode, Pid,
        namespace::nsproxy::NsProxy,
        posix_thread::ptrace::TraceeStatus,
        signal::{PauseReason, PollHandle, sig_mask::SigMask},
    },
    thread::{Thread, Tid},
    time::{Timer, TimerManager, clocks::ProfClock, timer::TimerFn},
};

pub(crate) mod alien_access;
mod builder;
mod cpu_sync;
mod exit;
pub(crate) mod futex;
mod personality;
mod posix_thread_ext;
pub(crate) mod ptrace;
mod robust_list;
mod thread_local;

pub(crate) use builder::PosixThreadBuilder;
pub(super) use exit::sigkill_other_threads;
pub(crate) use exit::{do_exit, do_exit_group};
pub(crate) use personality::Personality;
pub use posix_thread_ext::AsPosixThread;
pub(crate) use robust_list::RobustListHead;
#[cfg(target_arch = "x86_64")]
pub(crate) use thread_local::SuppUserContext;
pub(crate) use thread_local::{AsThreadLocal, FileTableRefMut, ThreadLocal};

pub struct PosixThread {
    process: Weak<Process>,
    task: Weak<Task>,

    tid: AtomicU32,

    name: Mutex<ThreadName>,

    credentials: Credentials,

    fs: RwMutex<Option<Arc<ThreadFsInfo>>>,

    file_table: Mutex<Option<RoArc<FileTable>>>,

    sig_mask: AtomicSigMask,
    sig_queues: SigQueues,
    signalled_waker: SpinLock<Option<(Arc<Waker>, PauseReason)>>,

    prof_clock: Arc<ProfClock>,
    virtual_timer_manager: Arc<TimerManager>,
    prof_timer_manager: Arc<TimerManager>,

    io_priority: AtomicU32,

    ns_proxy: Mutex<Option<Arc<NsProxy>>>,

    timer_slack_ns: AtomicU64,
    default_timer_slack_ns: AtomicU64,

    tracee_status: Once<TraceeStatus>,
    tracees: Once<Mutex<BTreeMap<Tid, Arc<Thread>>>>,

    exit_code: AtomicU32,

    personality: AtomicU32,
}

impl PosixThread {
    pub(crate) fn process(&self) -> Arc<Process> {
        self.process.upgrade().unwrap()
    }

    pub(crate) fn weak_process(&self) -> &Weak<Process> {
        &self.process
    }

    pub(crate) fn tid(&self) -> Tid {
        self.tid.load(Ordering::Relaxed)
    }

    pub(super) fn set_main(&self, pid: Pid) {
        debug_assert_eq!(pid, self.process.upgrade().unwrap().pid());
        debug_assert_ne!(pid, self.tid.load(Ordering::Relaxed));

        self.tid.store(pid, Ordering::Relaxed);
    }

    pub(crate) fn thread_name(&self) -> &Mutex<ThreadName> {
        &self.name
    }

    pub(crate) fn read_fs(&self) -> RwMutexReadGuard<'_, Option<Arc<ThreadFsInfo>>> {
        self.fs.read()
    }

    pub(in crate::process) fn set_fs(&self, new_fs: Arc<ThreadFsInfo>) {
        let mut fs_lock = self.fs.write();
        *fs_lock = Some(new_fs);
    }

    pub(crate) fn file_table(&self) -> &Mutex<Option<RoArc<FileTable>>> {
        &self.file_table
    }

    pub(crate) fn sig_mask(&self) -> SigMask {
        self.sig_mask.load(Ordering::Relaxed)
    }

    pub(super) fn sig_queues(&self) -> &SigQueues {
        &self.sig_queues
    }

    pub(crate) fn has_signal_blocked(&self, signum: SigNum) -> bool {
        self.sig_mask.contains(signum, Ordering::Relaxed)
    }

    pub(crate) fn set_signalled_waker(&self, waker: Arc<Waker>, reason: PauseReason) {
        let mut signalled_waker = self.signalled_waker.lock();
        assert!(signalled_waker.is_none());
        *signalled_waker = Some((waker, reason));
    }

    pub(crate) fn clear_signalled_waker(&self) {
        *self.signalled_waker.lock() = None;
    }

    pub(crate) fn sleeping_state(&self) -> SleepingState {
        let signalled_waker = self.signalled_waker.lock();
        let task = self.task.upgrade().unwrap();
        match (
            signalled_waker.as_ref(),
            task.schedule_info().cpu.get().is_none(),
        ) {
            (Some((_, PauseReason::Sleep)), true) => SleepingState::Interruptible,
            (Some((_, PauseReason::StopBySignal)), true) => SleepingState::StopBySignal,
            (Some((_, PauseReason::StopByPtrace)), true) => SleepingState::StopByPtrace,
            (None, true) => SleepingState::Uninterruptible,
            (_, false) => SleepingState::Running,
        }
    }

    pub(crate) fn wake_signalled_waker(&self) {
        if let Some((waker, _)) = &*self.signalled_waker.lock() {
            waker.wake_up();
        }
    }

    pub(crate) fn enqueue_signal(&self, signal: Box<dyn Signal>) {
        self.sig_queues.enqueue(signal);
        self.wake_signalled_waker();
    }

    pub(crate) fn register_signalfd_poller(&self, poller: &mut PollHandle, mask: IoEvents) {
        self.sig_queues.register_signalfd_poller(poller, mask);
        self.process()
            .sig_queues()
            .register_signalfd_poller(poller, mask);
    }

    pub(crate) fn prof_clock(&self) -> &Arc<ProfClock> {
        &self.prof_clock
    }

    pub(crate) fn create_prof_timer<F: TimerFn>(&self, func: F) -> Arc<Timer<F>> {
        self.prof_timer_manager.create_timer(func)
    }

    pub(crate) fn create_virtual_timer<F: TimerFn>(&self, func: F) -> Arc<Timer<F>> {
        self.virtual_timer_manager.create_timer(func)
    }

    pub(crate) fn process_expired_timers(&self) {
        self.prof_timer_manager.process_expired_timers();
    }

    pub(crate) fn credentials(&self) -> Credentials<ReadOp> {
        self.credentials.dup().restrict()
    }

    pub(crate) fn credentials_dup(&self) -> Credentials<ReadDupOp> {
        self.credentials.dup().restrict()
    }

    pub(crate) fn io_priority(&self) -> &AtomicU32 {
        &self.io_priority
    }

    pub(crate) fn ns_proxy(&self) -> &Mutex<Option<Arc<NsProxy>>> {
        &self.ns_proxy
    }

    pub(crate) fn timer_slack_ns(&self) -> u64 {
        self.timer_slack_ns.load(Ordering::Relaxed)
    }

    pub(crate) fn set_timer_slack_ns(&self, slack_ns: u64) {
        self.timer_slack_ns.store(slack_ns, Ordering::Relaxed);
    }

    pub(crate) fn reset_timer_slack_to_default(&self) {
        let default = self.default_timer_slack_ns.load(Ordering::Relaxed);
        self.timer_slack_ns.store(default, Ordering::Relaxed);
    }

    pub(super) fn set_exit_code(&self, exit_code: ExitCode) {
        self.exit_code.store(exit_code, Ordering::Relaxed);
    }

    pub(crate) fn exit_code(&self) -> ExitCode {
        self.exit_code.load(Ordering::Relaxed)
    }
}

pub(crate) trait ContextPthreadAdminApi {
    fn set_sig_mask(&self, sig_mask: SigMask);

    fn save_and_set_sig_mask(&self, sig_mask: SigMask);

    fn credentials_mut(&self) -> Credentials<ReadWriteOp>;
}

impl ContextPthreadAdminApi for Context<'_> {
    fn set_sig_mask(&self, mut sig_mask: SigMask) {
        use crate::process::signal::constants::{SIGKILL, SIGSTOP};

        sig_mask -= SIGKILL;
        sig_mask -= SIGSTOP;

        self.posix_thread
            .sig_mask
            .store(sig_mask, Ordering::Relaxed);
    }

    fn save_and_set_sig_mask(&self, sig_mask: SigMask) {
        let sig_mask_saved = self.thread_local.sig_mask_saved();
        debug_assert!(sig_mask_saved.get().is_none());
        sig_mask_saved.set(Some(self.posix_thread.sig_mask()));

        self.set_sig_mask(sig_mask);
    }

    fn credentials_mut(&self) -> Credentials<ReadWriteOp> {
        self.posix_thread.credentials.dup().restrict()
    }
}

const MAX_THREAD_NAME_LEN: usize = 16;
pub(crate) type ThreadName = FixedCStr<MAX_THREAD_NAME_LEN>;

pub(crate) fn derive_thread_name(exec_path: &str) -> ThreadName {
    let Some(path) = exec_path.split('/').next_back() else {
        return ThreadName::new_zeroed();
    };

    ThreadName::from_str_truncated(path)
}

pub(crate) const FIRST_POSIX_TID: Tid = 1;

static POSIX_TID_ALLOCATOR: AtomicU32 = AtomicU32::new(FIRST_POSIX_TID);

pub(crate) fn allocate_posix_tid() -> Tid {
    let tid = POSIX_TID_ALLOCATOR.fetch_add(1, Ordering::Relaxed);
    if tid >= PID_MAX {
        warn!("the allocated ID is greater than the maximum allowed PID");
    }
    tid
}

pub(crate) fn last_tid() -> Tid {
    POSIX_TID_ALLOCATOR.load(Ordering::Relaxed) - 1
}

pub(crate) const PID_MAX: u32 = u32::MAX / 2;

#[derive(Clone, Copy, Debug)]
pub(crate) enum SleepingState {
    Running,
    Interruptible,
    Uninterruptible,
    StopBySignal,
    StopByPtrace,
}
