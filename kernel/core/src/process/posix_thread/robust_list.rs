use ostd::{mm::VmIo, task::Task};

use crate::{
    context::current_userspace,
    prelude::*,
    process::posix_thread::futex::{FutexVisibility, futex_wake},
    thread::Tid,
};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct RobustList {
    next: Vaddr,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct RobustListHead {
    list: RobustList,
    futex_offset: isize,
    list_op_pending: Vaddr,
}

impl RobustListHead {
    pub(crate) fn futexes(&self) -> FutexIter<'_> {
        FutexIter::new(self)
    }

    fn pending_futex_addr(&self) -> Option<Vaddr> {
        if self.list_op_pending == 0 {
            None
        } else {
            self.futex_addr(self.list_op_pending)
        }
    }

    fn futex_addr(&self, entry_ptr: Vaddr) -> Option<Vaddr> {
        self.futex_offset
            .checked_add(entry_ptr as isize)
            .map(|result| result as Vaddr)
    }
}

pub(crate) struct FutexIter<'a> {
    robust_list: &'a RobustListHead,
    entry_ptr: Vaddr,
    count: isize,
}

impl<'a> FutexIter<'a> {
    pub(crate) fn new(robust_list: &'a RobustListHead) -> Self {
        Self {
            robust_list,
            entry_ptr: robust_list.list.next,
            count: 0,
        }
    }

    fn set_end(&mut self) {
        self.count = -1;
    }

    fn is_end(&self) -> bool {
        self.count < 0
    }
}

const ROBUST_LIST_LIMIT: isize = 2048;

impl Iterator for FutexIter<'_> {
    type Item = Vaddr;

    fn next(&mut self) -> Option<Self::Item> {
        if self.is_end() {
            return None;
        }

        let end_ptr = self.robust_list.list.next;
        while self.entry_ptr != end_ptr || self.count == 0 {
            if self.count == ROBUST_LIST_LIMIT {
                break;
            }
            if self.entry_ptr == 0 {
                return None;
            }
            let futex_addr = if self.entry_ptr != self.robust_list.list_op_pending {
                self.robust_list.futex_addr(self.entry_ptr)
            } else {
                None
            };
            let Ok(robust_list) = current_userspace!().read_val::<RobustList>(self.entry_ptr)
            else {
                return None;
            };
            self.entry_ptr = robust_list.next;
            self.count += 1;
            if futex_addr.is_some() {
                return futex_addr;
            }
        }
        self.set_end();
        self.robust_list.pending_futex_addr()
    }
}

const FUTEX_WAITERS: u32 = 0x8000_0000;
const FUTEX_OWNER_DIED: u32 = 0x4000_0000;
const FUTEX_TID_MASK: u32 = 0x3FFF_FFFF;

pub(crate) fn wake_robust_futex(futex_addr: Vaddr, tid: Tid) -> Result<()> {
    if !futex_addr.is_multiple_of(align_of::<u32>()) {
        return Err(errno!(
            EINVAL,
            "the futex word is not aligned on a four-byte boundary"
        ));
    }

    let task = Task::current().unwrap();
    let user_space = CurrentUserSpace::new(task.as_thread_local().unwrap());

    let (reader, writer) = user_space.reader_writer(futex_addr, size_of::<u32>())?;

    let mut old_val: u32 = reader.atomic_load()?;
    loop {
        if old_val & FUTEX_TID_MASK != tid {
            break;
        }

        let new_val = (old_val & FUTEX_WAITERS) | FUTEX_OWNER_DIED;
        match writer.atomic_compare_exchange(&reader, old_val, new_val)? {
            (cur_val, false) => old_val = cur_val,
            (_, true) => {
                if new_val & FUTEX_WAITERS != 0 {
                    debug!("wake the robust futex at {:#x}", futex_addr);
                    futex_wake(futex_addr, 1, FutexVisibility::Shared)?;
                }
                break;
            }
        }
    }

    Ok(())
}
