use alloc::{collections::VecDeque, sync::Arc};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use super::{LocalIrqDisabled, SpinLock};
use crate::task::{Task, scheduler};

pub struct WaitQueue {
    num_wakers: AtomicU32,
    wakers: SpinLock<VecDeque<Arc<Waker>>, LocalIrqDisabled>,
}

impl WaitQueue {
    pub const fn new() -> Self {
        WaitQueue {
            num_wakers: AtomicU32::new(0),
            wakers: SpinLock::new(VecDeque::new()),
        }
    }

    #[track_caller]
    pub fn wait_until<F, R>(&self, mut cond: F) -> R
    where
        F: FnMut() -> Option<R>,
    {
        if let Some(res) = cond() {
            return res;
        }

        let (waiter, _) = Waiter::new_pair();
        let cond = || {
            self.enqueue(waiter.waker());
            cond()
        };
        waiter
            .wait_until_or_cancelled(cond, || Ok::<(), ()>(()))
            .unwrap()
    }

    pub fn wake_one(&self) -> bool {
        if self.is_empty() {
            return false;
        }

        loop {
            let mut wakers = self.wakers.lock();
            let Some(waker) = wakers.pop_front() else {
                return false;
            };
            self.num_wakers.fetch_sub(1, Ordering::Release);
            drop(wakers);

            if waker.wake_up() {
                return true;
            }
        }
    }

    pub fn wake_all(&self) -> usize {
        if self.is_empty() {
            return 0;
        }

        let mut num_woken = 0;

        loop {
            let mut wakers = self.wakers.lock();
            let Some(waker) = wakers.pop_front() else {
                break;
            };
            self.num_wakers.fetch_sub(1, Ordering::Release);
            drop(wakers);

            if waker.wake_up() {
                num_woken += 1;
            }
        }

        num_woken
    }

    fn is_empty(&self) -> bool {
        self.num_wakers.fetch_add(0, Ordering::Release) == 0
    }

    #[doc(hidden)]
    pub fn enqueue(&self, waker: Arc<Waker>) {
        let mut wakers = self.wakers.lock();
        wakers.push_back(waker);
        self.num_wakers.fetch_add(1, Ordering::Acquire);
    }
}

impl Default for WaitQueue {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Waiter {
    waker: Arc<Waker>,
}

impl !Send for Waiter {}
impl !Sync for Waiter {}

pub struct Waker {
    has_woken: AtomicBool,
    task: Arc<Task>,
}

impl Waiter {
    pub fn new_pair() -> (Self, Arc<Waker>) {
        let waker = Arc::new(Waker {
            has_woken: AtomicBool::new(false),
            task: Task::current().unwrap().cloned(),
        });
        let waiter = Self {
            waker: waker.clone(),
        };
        (waiter, waker)
    }

    #[track_caller]
    pub fn wait(&self) {
        self.waker.do_wait();
    }

    #[track_caller]
    pub fn wait_until_or_cancelled<F, R, FCancel, E>(
        &self,
        mut cond: F,
        cancel_cond: FCancel,
    ) -> Result<R, E>
    where
        F: FnMut() -> Option<R>,
        FCancel: Fn() -> Result<(), E>,
    {
        loop {
            if let Some(res) = cond() {
                return Ok(res);
            };

            if let Err(e) = cancel_cond() {
                self.waker.close();
                return cond().ok_or(e);
            }

            self.wait();
        }
    }

    pub fn waker(&self) -> Arc<Waker> {
        self.waker.clone()
    }

    pub fn task(&self) -> &Arc<Task> {
        &self.waker.task
    }
}

impl Drop for Waiter {
    fn drop(&mut self) {
        self.waker.close();
    }
}

impl Waker {
    pub fn wake_up(&self) -> bool {
        if self.has_woken.swap(true, Ordering::Release) {
            return false;
        }
        scheduler::unpark_target(self.task.clone());

        true
    }

    #[track_caller]
    fn do_wait(&self) {
        while !self.has_woken.swap(false, Ordering::Acquire) {
            scheduler::park_current(|| self.has_woken.load(Ordering::Acquire));
        }
    }

    fn close(&self) {
        let _ = self.has_woken.swap(true, Ordering::Acquire);
    }
}

#[cfg(ktest)]
mod test {
    use super::*;
    use crate::{prelude::*, task::TaskOptions};

    fn queue_wake<F>(wake: F)
    where
        F: Fn(&WaitQueue) + Sync + Send + 'static,
    {
        let queue = Arc::new(WaitQueue::new());
        let queue_cloned = queue.clone();

        let cond = Arc::new(AtomicBool::new(false));
        let cond_cloned = cond.clone();

        TaskOptions::new(move || {
            Task::yield_now();

            cond_cloned.store(true, Ordering::Relaxed);
            wake(&queue_cloned);
        })
        .data(())
        .spawn()
        .unwrap();

        queue.wait_until(|| cond.load(Ordering::Relaxed).then_some(()));

        assert!(cond.load(Ordering::Relaxed));
    }

    #[ktest]
    fn queue_wake_one() {
        queue_wake(|queue| {
            queue.wake_one();
        });
    }

    #[ktest]
    fn queue_wake_all() {
        queue_wake(|queue| {
            queue.wake_all();
        });
    }

    #[ktest]
    fn waiter_wake_twice() {
        let (_waiter, waker) = Waiter::new_pair();

        assert!(waker.wake_up());
        assert!(!waker.wake_up());
    }

    #[ktest]
    fn waiter_wake_drop() {
        let (waiter, waker) = Waiter::new_pair();

        drop(waiter);
        assert!(!waker.wake_up());
    }

    #[ktest]
    fn waiter_wake_async() {
        let (waiter, waker) = Waiter::new_pair();

        let cond = Arc::new(AtomicBool::new(false));
        let cond_cloned = cond.clone();

        TaskOptions::new(move || {
            Task::yield_now();

            cond_cloned.store(true, Ordering::Relaxed);
            assert!(waker.wake_up());
        })
        .data(())
        .spawn()
        .unwrap();

        waiter.wait();

        assert!(cond.load(Ordering::Relaxed));
    }

    #[ktest]
    fn waiter_wake_reorder() {
        let (waiter, waker) = Waiter::new_pair();

        let cond = Arc::new(AtomicBool::new(false));
        let cond_cloned = cond.clone();

        let (waiter2, waker2) = Waiter::new_pair();

        let cond2 = Arc::new(AtomicBool::new(false));
        let cond2_cloned = cond2.clone();

        TaskOptions::new(move || {
            Task::yield_now();

            cond2_cloned.store(true, Ordering::Relaxed);
            assert!(waker2.wake_up());

            Task::yield_now();

            cond_cloned.store(true, Ordering::Relaxed);
            assert!(waker.wake_up());
        })
        .data(())
        .spawn()
        .unwrap();

        waiter.wait();
        assert!(cond.load(Ordering::Relaxed));

        waiter2.wait();
        assert!(cond2.load(Ordering::Relaxed));
    }
}
