use core::sync::atomic::{AtomicUsize, Ordering};

use super::{
    constants::*,
    sig_mask::{SigMask, SigSet},
    sig_num::SigNum,
    signals::Signal,
};
use crate::{
    events::IoEvents,
    prelude::*,
    process::signal::{PollHandle, Pollee, sig_disposition::SigDispositions},
};

pub(crate) struct SigQueues {
    count: AtomicUsize,
    queues: Mutex<Queues>,
    signalfd_pollee: Pollee,
}

impl SigQueues {
    pub(crate) fn new() -> Self {
        Self {
            count: AtomicUsize::new(0),
            queues: Mutex::new(Queues::new()),
            signalfd_pollee: Pollee::new(),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.count.load(Ordering::Relaxed) == 0
    }

    pub(crate) fn enqueue(&self, signal: Box<dyn Signal>) {
        let mut queues = self.queues.lock();
        if queues.enqueue(signal) {
            self.count.fetch_add(1, Ordering::Relaxed);
            drop(queues);
            self.signalfd_pollee.notify(IoEvents::IN);
        }
    }

    pub(crate) fn dequeue(&self, blocked: &SigMask) -> Option<Box<dyn Signal>> {
        if self.is_empty() {
            return None;
        }

        let mut queues = self.queues.lock();
        let signal = queues.dequeue(blocked);
        if signal.is_some() {
            self.count.fetch_sub(1, Ordering::Relaxed);
        }
        signal
    }

    pub(crate) fn sig_pending(&self) -> SigSet {
        let queues = self.queues.lock();
        queues.sig_pending()
    }

    pub(in crate::process) fn has_pending(
        &self,
        blocked: SigMask,
        sig_dispositions: &SigDispositions,
    ) -> bool {
        let mut queues = self.queues.lock();
        let (dequeued_signals, has_pending) = queues.has_pending(blocked, sig_dispositions);
        self.count.fetch_sub(dequeued_signals, Ordering::Relaxed);
        has_pending
    }

    pub(in crate::process) fn has_pending_signal(&self, signum: SigNum) -> bool {
        self.queues.lock().has_pending_signal(signum)
    }

    pub(in crate::process) fn register_signalfd_poller(
        &self,
        poller: &mut PollHandle,
        mask: IoEvents,
    ) {
        self.signalfd_pollee.register_poller(poller, mask);
    }
}

impl Default for SigQueues {
    fn default() -> Self {
        Self::new()
    }
}

struct Queues {
    std_queues: Vec<Option<Box<dyn Signal>>>,
    rt_queues: Vec<VecDeque<Box<dyn Signal>>>,
}

impl Queues {
    fn new() -> Self {
        let std_queues = (0..COUNT_STD_SIGS).map(|_| None).collect();
        let rt_queues = (0..COUNT_RT_SIGS).map(|_| Default::default()).collect();
        Self {
            std_queues,
            rt_queues,
        }
    }

    fn enqueue(&mut self, signal: Box<dyn Signal>) -> bool {
        let signum = signal.num();
        if signum.is_std() {
            let queue = self.get_std_queue_mut(signum);
            if queue.is_some() {
                return false;
            }
            *queue = Some(signal);
        } else {
            let queue = self.get_rt_queue_mut(signum);
            queue.push_back(signal);
        }

        true
    }

    fn dequeue(&mut self, blocked: &SigMask) -> Option<Box<dyn Signal>> {
        const ORDERED_STD_SIGS: [SigNum; COUNT_STD_SIGS] = [
            SIGKILL, SIGTERM, SIGSTOP, SIGSEGV, SIGILL, SIGHUP, SIGCONT, SIGINT, SIGQUIT, SIGTRAP,
            SIGABRT, SIGBUS, SIGFPE, SIGUSR1, SIGUSR2, SIGPIPE, SIGALRM, SIGSTKFLT, SIGCHLD,
            SIGTSTP, SIGTTIN, SIGTTOU, SIGURG, SIGXCPU, SIGXFSZ, SIGVTALRM, SIGPROF, SIGWINCH,
            SIGIO, SIGPWR, SIGSYS,
        ];
        for &signum in &ORDERED_STD_SIGS {
            if blocked.contains(signum) {
                continue;
            }

            let queue = self.get_std_queue_mut(signum);
            let signal = queue.take();
            if signal.is_some() {
                return signal;
            }
        }

        for signum in MIN_RT_SIG_NUM..=MAX_RT_SIG_NUM {
            let signum = SigNum::try_from(signum).unwrap();
            if blocked.contains(signum) {
                continue;
            }

            let queue = self.get_rt_queue_mut(signum);
            let signal = queue.pop_front();
            if signal.is_some() {
                return signal;
            }
        }

        None
    }

    fn has_pending(
        &mut self,
        blocked: SigMask,
        sig_dispositions: &SigDispositions,
    ) -> (usize, bool) {
        let mut dequeued_signals = 0;

        let has_pending = self.std_queues.iter_mut().any(|signal| {
            let Some(signal_ref) = signal.as_ref().map(AsRef::as_ref) else {
                return false;
            };

            if blocked.contains(signal_ref.num()) {
                return false;
            }

            if sig_dispositions.will_ignore(signal_ref) {
                dequeued_signals += 1;
                *signal = None;
                return false;
            }

            true
        }) || self.rt_queues.iter_mut().any(|rt_queue| {
            let Some(signal) = rt_queue.front() else {
                return false;
            };

            if blocked.contains(signal.num()) {
                return false;
            }

            if sig_dispositions.will_ignore(signal.as_ref()) {
                dequeued_signals += rt_queue.len();
                rt_queue.clear();
                return false;
            }

            true
        });

        (dequeued_signals, has_pending)
    }

    fn has_pending_signal(&self, signum: SigNum) -> bool {
        if signum.is_std() {
            self.get_std_queue(signum).is_some()
        } else if signum.is_real_time() {
            !self.get_rt_queue(signum).is_empty()
        } else {
            false
        }
    }

    fn get_std_queue(&self, signum: SigNum) -> &Option<Box<dyn Signal>> {
        debug_assert!(signum.is_std());
        let idx = (signum.as_u8() - MIN_STD_SIG_NUM) as usize;
        &self.std_queues[idx]
    }

    fn get_std_queue_mut(&mut self, signum: SigNum) -> &mut Option<Box<dyn Signal>> {
        debug_assert!(signum.is_std());
        let idx = (signum.as_u8() - MIN_STD_SIG_NUM) as usize;
        &mut self.std_queues[idx]
    }

    fn get_rt_queue(&self, signum: SigNum) -> &VecDeque<Box<dyn Signal>> {
        debug_assert!(signum.is_real_time());
        let idx = (signum.as_u8() - MIN_RT_SIG_NUM) as usize;
        &self.rt_queues[idx]
    }

    fn get_rt_queue_mut(&mut self, signum: SigNum) -> &mut VecDeque<Box<dyn Signal>> {
        debug_assert!(signum.is_real_time());
        let idx = (signum.as_u8() - MIN_RT_SIG_NUM) as usize;
        &mut self.rt_queues[idx]
    }

    fn sig_pending(&self) -> SigSet {
        let mut pending = SigSet::new_empty();

        for (idx, signal) in self.std_queues.iter().enumerate() {
            if signal.is_some() {
                pending += SigNum::from_u8(idx as u8 + MIN_STD_SIG_NUM);
            }
        }

        for (idx, signals) in self.rt_queues.iter().enumerate() {
            if !signals.is_empty() {
                pending += SigNum::from_u8(idx as u8 + MIN_RT_SIG_NUM);
            }
        }

        pending
    }
}
