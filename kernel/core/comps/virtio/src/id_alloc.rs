use id_alloc::IdAlloc;
use ostd::sync::{SpinLock, WaitQueue};

pub(crate) struct SyncIdAlloc {
    wait_queue: WaitQueue,
    id_allocator: SpinLock<IdAlloc>,
}

impl SyncIdAlloc {
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            wait_queue: WaitQueue::new(),
            id_allocator: SpinLock::new(IdAlloc::with_capacity(capacity)),
        }
    }

    pub(crate) fn alloc(&self) -> usize {
        self.wait_queue
            .wait_until(|| self.id_allocator.disable_irq().lock().alloc())
    }

    pub(crate) fn dealloc(&self, id: usize) {
        self.id_allocator.disable_irq().lock().free(id);
        self.wait_queue.wake_all();
    }
}

impl core::fmt::Debug for SyncIdAlloc {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("SyncIdAlloc")
            .field("id_allocator", &self.id_allocator.lock())
            .finish_non_exhaustive()
    }
}
