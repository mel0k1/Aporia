#![expect(dead_code)]

use intrusive_collections::linked_list::LinkedList;
use ostd::cpu::{CpuId, CpuSet};
use spin::Once;
use work_item::{WorkItem, WorkItemAdapter};
use worker_pool::WorkerPool;

use crate::prelude::*;

mod simple_scheduler;
pub(crate) mod work_item;
pub(crate) mod worker;
pub(crate) mod worker_pool;

static WORKERPOOL_NORMAL: Once<Arc<WorkerPool>> = Once::new();
static WORKERPOOL_HIGH_PRI: Once<Arc<WorkerPool>> = Once::new();
static WORKQUEUE_GLOBAL_NORMAL: Once<Arc<WorkQueue>> = Once::new();
static WORKQUEUE_GLOBAL_HIGH_PRI: Once<Arc<WorkQueue>> = Once::new();

pub(crate) fn submit_work_func<F>(work_func: F, work_priority: WorkPriority)
where
    F: Fn() + Send + Sync + 'static,
{
    let work_item = WorkItem::new(Box::new(work_func));
    submit_work_item(work_item, work_priority);
}

pub(crate) fn submit_work_item(work_item: Arc<WorkItem>, work_priority: WorkPriority) -> bool {
    match work_priority {
        WorkPriority::High => WORKQUEUE_GLOBAL_HIGH_PRI
            .get()
            .unwrap()
            .enqueue(work_item.clone()),
        WorkPriority::Normal => WORKQUEUE_GLOBAL_NORMAL
            .get()
            .unwrap()
            .enqueue(work_item.clone()),
    }
}

pub(crate) struct WorkQueue {
    worker_pool: Weak<WorkerPool>,
    inner: SpinLock<WorkQueueInner>,
}

struct WorkQueueInner {
    pending_work_items: LinkedList<WorkItemAdapter>,
}

impl WorkQueue {
    pub(crate) fn new(worker_pool: Weak<WorkerPool>) -> Arc<Self> {
        let queue = Arc::new(WorkQueue {
            worker_pool: worker_pool.clone(),
            inner: SpinLock::new(WorkQueueInner {
                pending_work_items: LinkedList::new(WorkItemAdapter::NEW),
            }),
        });
        worker_pool
            .upgrade()
            .unwrap()
            .assign_work_queue(queue.clone());
        queue
    }

    pub(crate) fn enqueue(&self, work_item: Arc<WorkItem>) -> bool {
        if !work_item.try_pending() {
            return false;
        }
        self.inner
            .disable_irq()
            .lock()
            .pending_work_items
            .push_back(work_item.clone());

        if let Some(worker_pool) = self.worker_pool.upgrade() {
            worker_pool.notify_work(work_item.cpu_affinity());
        }

        true
    }

    fn dequeue(&self, request_cpu: CpuId) -> Option<Arc<WorkItem>> {
        let mut inner = self.inner.disable_irq().lock();
        let mut cursor = inner.pending_work_items.front_mut();
        while let Some(item) = cursor.get() {
            if item.is_valid_cpu(request_cpu) {
                return cursor.remove();
            }

            cursor.move_next();
        }

        None
    }

    fn has_pending_work_items(&self, request_cpu: CpuId) -> bool {
        self.inner
            .disable_irq()
            .lock()
            .pending_work_items
            .iter()
            .any(|item| item.is_valid_cpu(request_cpu))
    }
}

pub(crate) fn init_in_first_kthread() {
    WORKERPOOL_NORMAL.call_once(|| {
        let cpu_set = CpuSet::new_full();
        WorkerPool::new(WorkPriority::Normal, cpu_set)
    });
    WORKERPOOL_NORMAL.get().unwrap().run();
    WORKERPOOL_HIGH_PRI.call_once(|| {
        let cpu_set = CpuSet::new_full();
        WorkerPool::new(WorkPriority::High, cpu_set)
    });
    WORKERPOOL_HIGH_PRI.get().unwrap().run();
    WORKQUEUE_GLOBAL_NORMAL
        .call_once(|| WorkQueue::new(Arc::downgrade(WORKERPOOL_NORMAL.get().unwrap())));
    WORKQUEUE_GLOBAL_HIGH_PRI
        .call_once(|| WorkQueue::new(Arc::downgrade(WORKERPOOL_HIGH_PRI.get().unwrap())));
}

impl Drop for WorkQueue {
    fn drop(&mut self) {}
}

#[derive(PartialEq)]
pub(crate) enum WorkPriority {
    High,
    Normal,
}
