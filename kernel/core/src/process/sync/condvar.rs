#![expect(dead_code)]
#![expect(unused_variables)]

use alloc::sync::Arc;
use core::time::Duration;

use ostd::sync::{MutexGuard, SpinLock, WaitQueue};

use crate::time::wait::WaitTimeout;

pub(crate) enum LockErr<Guard> {
    Timeout(Guard),
    Unknown(Guard),
}

pub(crate) type LockResult<Guard> = Result<Guard, LockErr<Guard>>;

impl<Guard> LockErr<Guard> {
    pub(crate) fn into_guard(self) -> Guard {
        match self {
            LockErr::Timeout(guard) => guard,
            LockErr::Unknown(guard) => guard,
        }
    }
}

pub(crate) struct Condvar {
    waitqueue: Arc<WaitQueue>,
    counter: SpinLock<Inner>,
}

struct Inner {
    waiter_count: u64,
    notify_count: u64,
}

impl Condvar {
    pub(crate) fn new() -> Self {
        Condvar {
            waitqueue: Arc::new(WaitQueue::new()),
            counter: SpinLock::new(Inner {
                waiter_count: 0,
                notify_count: 0,
            }),
        }
    }

    pub(crate) fn wait<'a, T>(&self, guard: MutexGuard<'a, T>) -> LockResult<MutexGuard<'a, T>> {
        let cond = || {
            let mut counter = self.counter.lock();
            if counter.notify_count > 0 {
                counter.notify_count -= 1;
                Some(())
            } else {
                None
            }
        };
        {
            let mut counter = self.counter.lock();
            counter.waiter_count += 1;
        }
        let lock = MutexGuard::get_lock(&guard);
        drop(guard);
        self.waitqueue.wait_until(cond);
        Ok(lock.lock())
    }

    pub(crate) fn wait_timeout<'a, T>(
        &self,
        guard: MutexGuard<'a, T>,
        timeout: Duration,
    ) -> LockResult<(MutexGuard<'a, T>, bool)> {
        let cond = || {
            let mut counter = self.counter.lock();
            if counter.notify_count > 0 {
                counter.notify_count -= 1;
                Some(())
            } else {
                None
            }
        };
        {
            let mut counter = self.counter.lock();
            counter.waiter_count += 1;
        }
        let lock = MutexGuard::get_lock(&guard);
        drop(guard);
        let res = self.waitqueue.wait_until_or_timeout(cond, &timeout);
        match res {
            Ok(()) => Ok((lock.lock(), false)),
            Err(_) => {
                self.counter.lock().waiter_count -= 1;
                Err(LockErr::Timeout((lock.lock(), true)))
            }
        }
    }

    pub(crate) fn wait_timeout_while<'a, T, F>(
        &self,
        mut guard: MutexGuard<'a, T>,
        timeout: Duration,
        mut condition: F,
    ) -> LockResult<(MutexGuard<'a, T>, bool)>
    where
        F: FnMut(&mut T) -> bool,
    {
        loop {
            if !condition(&mut *guard) {
                return Ok((guard, false));
            }
            guard = match self.wait_timeout(guard, timeout) {
                Ok((guard, timeout_flag)) => guard,
                Err(LockErr::Timeout((guard, timeout_flag))) => {
                    return Err(LockErr::Timeout((guard, timeout_flag)));
                }
                Err(LockErr::Unknown(guard)) => return Err(LockErr::Unknown(guard)),
            }
        }
    }

    pub(crate) fn wait_while<'a, T, F>(
        &self,
        mut guard: MutexGuard<'a, T>,
        mut condition: F,
    ) -> LockResult<MutexGuard<'a, T>>
    where
        F: FnMut(&mut T) -> bool,
    {
        loop {
            if !condition(&mut *guard) {
                return Ok(guard);
            }
            guard = match self.wait(guard) {
                Ok(guard) => guard,
                Err(LockErr::Unknown(guard)) => return Err(LockErr::Unknown(guard)),
                _ => unreachable!(),
            }
        }
    }

    pub(crate) fn notify_one(&self) {
        let mut counter = self.counter.lock();
        if counter.waiter_count == 0 {
            return;
        }
        counter.notify_count += 1;
        self.waitqueue.wake_one();
        counter.waiter_count -= 1;
    }

    pub(crate) fn notify_all(&self) {
        let mut counter = self.counter.lock();
        if counter.waiter_count == 0 {
            return;
        }
        counter.notify_count = counter.waiter_count;
        self.waitqueue.wake_all();
        counter.waiter_count = 0;
    }
}

#[cfg(ktest)]
mod test {
    use ostd::{prelude::*, sync::Mutex};

    use super::*;
    use crate::thread::{Thread, kernel_thread::ThreadOptions};

    #[ktest]
    fn wait() {
        let pair = Arc::new((Mutex::new(false), Condvar::new()));
        let pair2 = Arc::clone(&pair);

        ThreadOptions::new(move || {
            Thread::yield_now();
            let (lock, cvar) = &*pair2;
            let mut started = lock.lock();
            *started = true;
            cvar.notify_one();
        })
        .spawn();

        {
            let (lock, cvar) = &*pair;
            let mut started = lock.lock();
            while !*started {
                started = cvar.wait(started).unwrap_or_else(|err| err.into_guard());
            }
            assert!(*started);
        }
    }

    #[ktest]
    fn wait_timeout() {
        let pair = Arc::new((Mutex::new(false), Condvar::new()));
        let pair2 = Arc::clone(&pair);

        ThreadOptions::new(move || {
            Thread::yield_now();
            let (lock, cvar) = &*pair2;
            let mut started = lock.lock();
            *started = true;
            cvar.notify_one();
        })
        .spawn();

        {
            let (lock, cvar) = &*pair;
            let mut started = lock.lock();
            while !*started {
                (started, _) = cvar
                    .wait_timeout(started, Duration::from_secs(1))
                    .unwrap_or_else(|err| err.into_guard());
            }
            assert!(*started);
        }
    }

    #[ktest]
    fn wait_while() {
        let pair = Arc::new((Mutex::new(true), Condvar::new()));
        let pair2 = Arc::clone(&pair);

        ThreadOptions::new(move || {
            Thread::yield_now();
            let (lock, cvar) = &*pair2;
            let mut started = lock.lock();
            *started = false;
            cvar.notify_one();
        })
        .spawn();

        {
            let (lock, cvar) = &*pair;
            let started = cvar
                .wait_while(lock.lock(), |started| *started)
                .unwrap_or_else(|err| err.into_guard());
            assert!(!*started);
        }
    }

    #[ktest]
    fn wait_timeout_while() {
        let pair = Arc::new((Mutex::new(true), Condvar::new()));
        let pair2 = Arc::clone(&pair);

        ThreadOptions::new(move || {
            Thread::yield_now();
            let (lock, cvar) = &*pair2;
            let mut started = lock.lock();
            *started = false;
            cvar.notify_one();
        })
        .spawn();

        {
            let (lock, cvar) = &*pair;
            let (started, _) = cvar
                .wait_timeout_while(lock.lock(), Duration::from_secs(1), |started| *started)
                .unwrap_or_else(|err| err.into_guard());
            assert!(!*started);
        }
    }
}
