use core::fmt;

use ostd::sync::{RwMutexWriteGuard, WaitQueue, Waiter, Waker};
pub(crate) use range::{FileRange, OFFSET_MAX};
use range::{FileRangeChange, OverlapWith};

use crate::{prelude::*, process::Pid};

mod range;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RangeLockOwner(usize);

impl RangeLockOwner {
    pub(in crate::fs) fn from_address(address: usize) -> Self {
        Self(address)
    }
}

#[derive(Clone, Debug)]
struct RangeLock {
    owner: RangeLockOwner,
    pid: Pid,
    type_: RangeLockType,
    range: FileRange,
}

pub(crate) struct RangeLockItem {
    lock: RangeLock,
    waitqueue: Arc<WaitQueue>,
}

impl RangeLockItem {
    pub(crate) fn new(
        owner: RangeLockOwner,
        pid: Pid,
        type_: RangeLockType,
        range: FileRange,
    ) -> Self {
        let lock = RangeLock {
            owner,
            pid,
            type_,
            range,
        };
        Self {
            lock,
            waitqueue: Arc::new(WaitQueue::new()),
        }
    }

    pub(crate) fn owner(&self) -> RangeLockOwner {
        self.lock.owner
    }

    pub(crate) fn set_owner(&mut self, owner: RangeLockOwner) {
        self.lock.owner = owner;
    }

    pub(crate) fn pid(&self) -> Pid {
        self.lock.pid
    }

    pub(crate) fn set_pid(&mut self, pid: Pid) {
        self.lock.pid = pid;
    }

    pub(crate) fn type_(&self) -> RangeLockType {
        self.lock.type_
    }

    pub(crate) fn set_type(&mut self, type_: RangeLockType) {
        self.lock.type_ = type_;
    }

    pub(crate) fn range(&self) -> FileRange {
        self.lock.range
    }

    pub(crate) fn set_range(&mut self, range: FileRange) {
        self.lock.range = range;
    }

    pub(crate) fn conflict_with(&self, other: &Self) -> bool {
        if self.owner() == other.owner() {
            return false;
        }
        if self.overlap_with(other).is_none() {
            return false;
        }
        if self.type_() == RangeLockType::WriteLock || other.type_() == RangeLockType::WriteLock {
            return true;
        }
        false
    }

    pub(crate) fn overlap_with(&self, other: &Self) -> Option<OverlapWith> {
        self.range().overlap_with(&other.range())
    }

    pub(crate) fn merge_with(&mut self, other: &Self) {
        self.lock
            .range
            .merge(&other.range())
            .expect("merge range failed");
    }

    pub(crate) fn start(&self) -> usize {
        self.range().start()
    }

    pub(crate) fn end(&self) -> usize {
        self.range().end()
    }

    pub(crate) fn set_start(&mut self, new_start: usize) {
        let change = self
            .lock
            .range
            .set_start(new_start)
            .expect("invalid new start");
        if let FileRangeChange::Shrunk = change {
            self.waitqueue.wake_all();
        }
    }

    pub(crate) fn set_end(&mut self, new_end: usize) {
        let change = self.lock.range.set_end(new_end).expect("invalid new end");
        if let FileRangeChange::Shrunk = change {
            self.waitqueue.wake_all();
        }
    }
}

impl Drop for RangeLockItem {
    fn drop(&mut self) {
        self.waitqueue.wake_all();
    }
}

impl Debug for RangeLockItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RangeLock")
            .field("owner", &self.owner())
            .field("pid", &self.pid())
            .field("type_", &self.type_())
            .field("range", &self.range())
            .finish()
    }
}

impl Clone for RangeLockItem {
    fn clone(&self) -> Self {
        Self {
            lock: self.lock.clone(),
            waitqueue: self.waitqueue.clone(),
        }
    }
}

pub(crate) struct RangeLockList {
    inner: RwMutex<Vec<RangeLockItem>>,
}

impl RangeLockList {
    pub(crate) fn new() -> Self {
        Self {
            inner: RwMutex::new(Vec::new()),
        }
    }

    pub(crate) fn test_lock(&self, lock: RangeLockItem) -> RangeLockItem {
        debug!("test_lock with RangeLock: {:?}", lock);
        let mut req_lock = lock.clone();
        let list = self.inner.read();
        for existing_lock in list.iter() {
            if lock.conflict_with(existing_lock) {
                req_lock.set_owner(existing_lock.owner());
                req_lock.set_pid(existing_lock.pid());
                req_lock.set_type(existing_lock.type_());
                req_lock.set_range(existing_lock.range());
                return req_lock;
            }
        }
        req_lock.set_type(RangeLockType::Unlock);
        req_lock
    }

    fn try_set_lock(&self, req_lock: &RangeLockItem, waker: Option<&Arc<Waker>>) -> Result<()> {
        let mut list = self.inner.write();
        if let Some(conflict_lock) = list.iter().find(|l| req_lock.conflict_with(l)) {
            if let Some(waker) = waker {
                conflict_lock.waitqueue.enqueue(waker.clone());
            }
            Err(errno!(EAGAIN, "the file is locked"))
        } else {
            Self::insert_lock_into_list(&mut list, req_lock);
            Ok(())
        }
    }

    pub(crate) fn set_lock(&self, req_lock: &RangeLockItem, is_nonblocking: bool) -> Result<()> {
        debug!(
            "set_lock with RangeLock: {:?}, is_nonblocking: {}",
            req_lock, is_nonblocking
        );
        if is_nonblocking {
            self.try_set_lock(req_lock, None)
        } else {
            let (waiter, waker) = Waiter::new_pair();
            waiter.pause_until(|| {
                let result = self.try_set_lock(req_lock, Some(&waker));
                if result.is_err_and(|err| err.error() == Errno::EAGAIN) {
                    None
                } else {
                    Some(result)
                }
            })?
        }
    }

    fn insert_lock_into_list(
        list: &mut RwMutexWriteGuard<Vec<RangeLockItem>>,
        lock: &RangeLockItem,
    ) {
        let first_same_owner_idx = match list.iter().position(|lk| lk.owner() == lock.owner()) {
            Some(idx) => idx,
            None => {
                list.push(lock.clone());
                return;
            }
        };
        list.insert(first_same_owner_idx, lock.clone());
        let mut pre_idx = first_same_owner_idx;
        let mut next_idx = pre_idx + 1;
        loop {
            if next_idx >= list.len() {
                break;
            }

            let (left, right) = list.split_at_mut(next_idx);
            let pre_lock = &mut left[pre_idx];
            let next_lock = &mut right[0];

            if next_lock.owner() != pre_lock.owner() {
                break;
            }
            if next_lock.type_() == pre_lock.type_() {
                if pre_lock.end() < next_lock.start() {
                    break;
                } else if next_lock.end() < pre_lock.start() {
                    list.swap(pre_idx, next_idx);
                    pre_idx += 1;
                    next_idx += 1;
                } else {
                    next_lock.merge_with(pre_lock);
                    list.remove(pre_idx);
                }
            } else {
                if pre_lock.end() <= next_lock.start() {
                    break;
                } else if next_lock.end() <= pre_lock.start() {
                    list.swap(pre_idx, next_idx);
                    pre_idx += 1;
                    next_idx += 1;
                } else {
                    let overlap_with = pre_lock.overlap_with(next_lock).unwrap();
                    match overlap_with {
                        OverlapWith::ToLeft => {
                            next_lock.set_start(pre_lock.end());
                            break;
                        }
                        OverlapWith::InMiddle => {
                            let right_lk = {
                                let mut r_lk = next_lock.clone();
                                r_lk.set_start(pre_lock.end());
                                r_lk
                            };
                            next_lock.set_end(pre_lock.start());
                            list.swap(pre_idx, next_idx);
                            list.insert(next_idx + 1, right_lk);
                            break;
                        }
                        OverlapWith::ToRight => {
                            next_lock.set_end(pre_lock.start());
                            list.swap(pre_idx, next_idx);
                            pre_idx += 1;
                            next_idx += 1;
                        }
                        OverlapWith::Includes => {
                            list.remove(next_idx);
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn unlock(&self, lock: &RangeLockItem) {
        debug!("unlock with RangeLock: {:?}", lock);
        let mut list = self.inner.write();
        let mut skipped = 0;
        while let Some(idx) = list
            .iter()
            .skip(skipped)
            .position(|lk| lk.owner() == lock.owner())
        {
            let idx = idx + skipped;
            let existing_lock = &mut list[idx];

            let overlap_with = match lock.overlap_with(existing_lock) {
                Some(overlap) => overlap,
                None => {
                    skipped = idx + 1;
                    continue;
                }
            };

            match overlap_with {
                OverlapWith::ToLeft => {
                    existing_lock.set_start(lock.end());
                    break;
                }
                OverlapWith::InMiddle => {
                    let right_lk = {
                        let mut r_lk = existing_lock.clone();
                        r_lk.set_start(lock.end());
                        r_lk
                    };
                    existing_lock.set_end(lock.start());
                    list.insert(idx + 1, right_lk);
                    break;
                }
                OverlapWith::ToRight => {
                    existing_lock.set_end(lock.start());
                    skipped = idx + 1;
                }
                OverlapWith::Includes => {
                    list.remove(idx);
                    skipped = idx;
                }
            }
        }
    }

    pub(crate) fn unlock_all(&self, owner: RangeLockOwner) {
        let mut list = self.inner.write();
        list.retain(|lock| lock.owner() != owner);
    }
}

impl Default for RangeLockList {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(u16)]
#[derive(Clone, Copy, Debug, PartialEq, TryFromInt)]
pub(crate) enum RangeLockType {
    ReadLock = 0,
    WriteLock = 1,
    Unlock = 2,
}
