use alloc::vec::Vec;
use core::{
    ops::DerefMut,
    sync::atomic::{Ordering, fence},
};

use aster_util::{field_ptr, safe_ptr::SafePtr};
use ostd::{
    mm::{HasDaddr, dma::DmaCoherent},
    warn,
};

use crate::{
    nvme_regs::NvmeDoorbellRegs,
    nvme_spec::{NvmeCommand, NvmeCompletion},
    transport::pci::transport::DbregAccess,
};

pub(crate) const QUEUE_DEPTH: usize = 64;

pub(crate) const QUEUE_NUM: usize = 2;

#[derive(Debug)]
pub(crate) struct NvmeCompletionQueue {
    cqueue: SafePtr<Cqring, DmaCoherent>,
    head: u16,
    phase: bool,
}

struct Cqring {
    ring: [NvmeCompletion; QUEUE_DEPTH],
}

impl NvmeCompletionQueue {
    pub(crate) fn new() -> Option<Self> {
        let dma = DmaCoherent::alloc(1, true).ok()?;
        Some(Self {
            cqueue: SafePtr::new(dma, 0),
            head: 0,
            phase: true,
        })
    }

    pub(crate) fn cq_daddr(&self) -> usize {
        self.cqueue.daddr()
    }

    fn complete(&mut self) -> Option<(u16, NvmeCompletion)> {
        let ring_ptr: SafePtr<[NvmeCompletion; QUEUE_DEPTH], &DmaCoherent> =
            field_ptr!(&self.cqueue, Cqring, ring);
        let mut ring_slot_ptr = ring_ptr.cast::<NvmeCompletion>();
        ring_slot_ptr.add(self.head as usize);

        let phase_tag = NvmeCompletion::read_phase_tag(&ring_slot_ptr);
        if phase_tag != self.phase {
            return None;
        }

        fence(Ordering::SeqCst);

        let entry = ring_slot_ptr
            .read()
            .expect("CQ slot pointer must be valid within allocated DMA ring");
        self.head = (self.head + 1) % (QUEUE_DEPTH as u16);
        if self.head == 0 {
            self.phase = !self.phase;
        }
        Some((self.head, entry))
    }
}

pub(crate) struct NvmeSubmissionQueue<C = NoContext> {
    inner: NvmeSubmissionQueueInner,
    context: C,
}

#[derive(Default)]
pub(crate) struct NoContext;

pub(crate) struct Context<T>([Option<T>; QUEUE_DEPTH]);

impl<T> Default for Context<T> {
    fn default() -> Self {
        Self(core::array::from_fn(|_| None))
    }
}

impl<C> core::fmt::Debug for NvmeSubmissionQueue<C> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NvmeSubmissionQueue")
            .field("inner", &self.inner)
            .finish_non_exhaustive()
    }
}

impl<C: Default> NvmeSubmissionQueue<C> {
    pub(crate) fn new() -> Option<Self> {
        Some(Self {
            inner: NvmeSubmissionQueueInner::new()?,
            context: Default::default(),
        })
    }
}

impl<C> NvmeSubmissionQueue<C> {
    pub(crate) fn update_sq_head(&mut self, completion: &NvmeCompletion) {
        self.inner.head = completion.sq_head() % (QUEUE_DEPTH as u16);
    }

    pub(crate) fn sq_daddr(&self) -> usize {
        self.inner.squeue.daddr()
    }

    pub(crate) fn free_slots(&self) -> usize {
        let used = if self.inner.tail >= self.inner.head {
            (self.inner.tail - self.inner.head) as usize
        } else {
            QUEUE_DEPTH - (self.inner.head - self.inner.tail) as usize
        };
        (QUEUE_DEPTH - 1) - used
    }
}

impl NvmeSubmissionQueue<NoContext> {
    pub(crate) fn with_items<T>(self) -> NvmeSubmissionQueue<Context<T>> {
        NvmeSubmissionQueue {
            inner: self.inner,
            context: Default::default(),
        }
    }
}

impl<T> NvmeSubmissionQueue<Context<T>> {
    pub(crate) fn take_item(&mut self, cid: u16) -> Option<T> {
        self.context.0.get_mut(cid as usize)?.take()
    }
}

#[derive(Debug)]
struct NvmeSubmissionQueueInner {
    squeue: SafePtr<Sqring, DmaCoherent>,
    tail: u16,
    head: u16,
}

struct Sqring {
    ring: [NvmeCommand; QUEUE_DEPTH],
}

impl NvmeSubmissionQueueInner {
    fn new() -> Option<Self> {
        let dma = DmaCoherent::alloc(1, true).ok()?;
        Some(Self {
            squeue: SafePtr::new(dma, 0),
            tail: 0,
            head: 0,
        })
    }

    fn submit(&mut self, entry: NvmeCommand) -> Option<u16> {
        let next_tail = (self.tail + 1) % (QUEUE_DEPTH as u16);
        if next_tail == self.head {
            return None;
        }

        let ring_ptr: SafePtr<[NvmeCommand; QUEUE_DEPTH], &DmaCoherent> =
            field_ptr!(&self.squeue, Sqring, ring);
        let mut ring_slot_ptr = ring_ptr.cast::<NvmeCommand>();
        ring_slot_ptr.add(self.tail as usize);
        ring_slot_ptr
            .write(&entry)
            .expect("SQ slot pointer must be valid within allocated DMA ring");

        self.tail = next_tail;
        Some(self.tail)
    }
}

pub(crate) struct NvmeCompletionQueueAccess<'a, Q> {
    qid: u16,
    dstrd: u16,
    queue: Q,
    dbregs: DbregAccess<'a>,
}

impl<'a, Q> NvmeCompletionQueueAccess<'a, Q>
where
    Q: DerefMut<Target = NvmeCompletionQueue>,
{
    pub(crate) fn new(qid: u16, dstrd: u16, queue: Q, dbregs: DbregAccess<'a>) -> Self {
        Self {
            qid,
            dstrd,
            queue,
            dbregs,
        }
    }

    pub(crate) fn complete(&mut self) -> Vec<NvmeCompletion> {
        let mut entries = Vec::new();
        let mut last_head = None;
        while let Some((entry, new_head)) = self.poll() {
            last_head = Some(new_head);
            entries.push(entry);
        }
        if let Some(new_head) = last_head {
            self.ring_doorbell(new_head);
        }
        entries
    }

    fn poll(&mut self) -> Option<(NvmeCompletion, u16)> {
        let (new_head, entry) = self.queue.complete()?;
        if entry.has_error() {
            warn!(
                "completion queue {}: command failed (CID={}, status={:04X}, SC={:#04x}, SQID={})",
                self.qid,
                entry.cid(),
                entry.status(),
                entry.status_code(),
                entry.sq_id(),
            );
        }
        Some((entry, new_head))
    }

    fn ring_doorbell(&mut self, new_head: u16) {
        fence(Ordering::SeqCst);
        self.dbregs.write_racy(
            NvmeDoorbellRegs::Cqhdbl,
            self.qid,
            self.dstrd,
            new_head as u32,
        );
    }
}

pub(crate) struct NvmeSubmissionQueueAccess<'a, Q> {
    qid: u16,
    dstrd: u16,
    queue: Q,
    dbregs: DbregAccess<'a>,
}

impl<'a, Q, C> NvmeSubmissionQueueAccess<'a, Q>
where
    Q: DerefMut<Target = NvmeSubmissionQueue<C>>,
{
    pub(crate) fn new(qid: u16, dstrd: u16, queue: Q, dbregs: DbregAccess<'a>) -> Self {
        Self {
            qid,
            dstrd,
            queue,
            dbregs,
        }
    }

    pub(crate) fn free_slots(&self) -> usize {
        self.queue.free_slots()
    }

    fn ring_doorbell(&mut self) {
        let new_tail = self.queue.inner.tail;
        fence(Ordering::SeqCst);
        self.dbregs.write_racy(
            NvmeDoorbellRegs::Sqtdbl,
            self.qid,
            self.dstrd,
            new_tail as u32,
        );
    }
}

impl<'a, Q> NvmeSubmissionQueueAccess<'a, Q>
where
    Q: DerefMut<Target = NvmeSubmissionQueue<NoContext>>,
{
    pub(crate) fn submit(&mut self, mut entry: NvmeCommand) -> Option<u16> {
        let cid = self.queue.inner.tail;
        entry.set_cid(cid);
        self.queue.inner.submit(entry)?;
        self.ring_doorbell();
        Some(cid)
    }
}

impl<'a, Q, T> NvmeSubmissionQueueAccess<'a, Q>
where
    Q: DerefMut<Target = NvmeSubmissionQueue<Context<T>>>,
{
    pub(crate) fn submit_with_items(
        &mut self,
        commands: impl IntoIterator<Item = (NvmeCommand, T)>,
    ) -> Option<usize> {
        let mut count = 0;
        for (entry, item) in commands {
            self.enqueue_one(entry, item)?;
            count += 1;
        }
        if count > 0 {
            self.ring_doorbell();
        }
        Some(count)
    }

    fn enqueue_one(&mut self, mut entry: NvmeCommand, item: T) -> Option<u16> {
        let cid = self.queue.inner.tail;
        if self.queue.context.0[cid as usize].is_some() {
            warn!(
                "submission queue {} slot {} is still outstanding",
                self.qid, cid
            );
            return None;
        }
        entry.set_cid(cid);
        self.queue.inner.submit(entry)?;
        self.queue.context.0[cid as usize] = Some(item);
        Some(cid)
    }
}
