use alloc::fmt;
use core::ptr;

use ostd::sync::{WaitQueue, Waiter, Waker};

use super::{FileLike, InodeHandle};
use crate::prelude::*;

#[derive(Clone, Debug)]
struct Flock {
    owner: Weak<dyn FileLike>,
    type_: FlockType,
}

pub(crate) struct FlockItem {
    lock: Flock,
    waitqueue: Arc<WaitQueue>,
}

impl FlockItem {
    pub(crate) fn new(owner: &Arc<dyn FileLike>, type_: FlockType) -> Self {
        Self {
            lock: Flock {
                owner: Arc::downgrade(owner),
                type_,
            },
            waitqueue: Arc::new(WaitQueue::new()),
        }
    }

    pub(crate) fn owner(&self) -> Option<Arc<dyn FileLike>> {
        Weak::upgrade(&self.lock.owner)
    }

    pub(crate) fn same_owner_with(&self, other: &Self) -> bool {
        self.lock.owner.ptr_eq(&other.lock.owner)
    }

    pub(crate) fn conflict_with(&self, other: &Self) -> bool {
        if self.same_owner_with(other) {
            return false;
        }
        if self.lock.type_ == FlockType::ExclusiveLock
            || other.lock.type_ == FlockType::ExclusiveLock
        {
            return true;
        }
        false
    }

    pub(crate) fn wake_all(&self) {
        self.waitqueue.wake_all();
    }
}

impl Clone for FlockItem {
    fn clone(&self) -> Self {
        Self {
            lock: self.lock.clone(),
            waitqueue: self.waitqueue.clone(),
        }
    }
}

impl Drop for FlockItem {
    fn drop(&mut self) {
        self.waitqueue.wake_all();
    }
}

impl Debug for FlockItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Flock")
            .field("owner", &self.lock.owner.as_ptr())
            .field("type_", &self.lock.type_)
            .finish()
    }
}

pub(crate) struct FlockList {
    inner: Mutex<Vec<FlockItem>>,
}

impl FlockList {
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(Vec::new()),
        }
    }

    fn try_set_lock(&self, req_lock: &FlockItem, waker: Option<&Arc<Waker>>) -> Result<()> {
        let mut list = self.inner.lock();
        if let Some(conflict_lock) = list.iter().find(|l| req_lock.conflict_with(l)) {
            if let Some(waker) = waker {
                conflict_lock.waitqueue.enqueue(waker.clone());
            }
            Err(errno!(EAGAIN, "the file is locked"))
        } else {
            match list.iter().position(|l| req_lock.same_owner_with(l)) {
                Some(idx) => {
                    list[idx] = req_lock.clone();
                }
                None => {
                    list.push(req_lock.clone());
                }
            }
            Ok(())
        }
    }

    pub(crate) fn set_lock(&self, req_lock: FlockItem, is_nonblocking: bool) -> Result<()> {
        debug!(
            "set_lock with Flock: {:?}, is_nonblocking: {}",
            req_lock, is_nonblocking
        );
        if is_nonblocking {
            self.try_set_lock(&req_lock, None)
        } else {
            let (waiter, waker) = Waiter::new_pair();
            waiter.pause_until(|| {
                let result = self.try_set_lock(&req_lock, Some(&waker));
                if result.is_err_and(|err| err.error() == Errno::EAGAIN) {
                    None
                } else {
                    Some(result)
                }
            })?
        }
    }

    pub(crate) fn unlock(&self, req_owner: &InodeHandle) {
        debug!("unlock with owner: {:?}", req_owner as *const InodeHandle);
        let mut list = self.inner.lock();
        list.retain(|lock| {
            if let Some(owner) = lock.owner() {
                if ptr::eq(
                    Arc::as_ptr(&owner) as *const InodeHandle,
                    req_owner as *const InodeHandle,
                ) {
                    lock.wake_all();
                    false
                } else {
                    true
                }
            } else {
                false
            }
        });
    }
}

impl Default for FlockList {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(u16)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum FlockType {
    SharedLock = 0,
    ExclusiveLock = 1,
}
