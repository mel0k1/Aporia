use alloc::collections::VecDeque;
use core::{
    any::{Any, TypeId},
    ptr::NonNull,
    sync::atomic::{AtomicUsize, Ordering},
};

use hashbrown::HashMap;

use crate::{
    Buf,
    os::{Mutex, MutexGuard},
    prelude::*,
};

pub(super) struct BioReqQueue {
    queue: Mutex<VecDeque<BioReq>>,
    num_reqs: AtomicUsize,
}

impl BioReqQueue {
    pub(super) fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            num_reqs: AtomicUsize::new(0),
        }
    }

    pub(super) fn enqueue(&self, req: BioReq) -> Result<()> {
        req.submit();
        self.queue.lock().push_back(req);
        self.num_reqs.fetch_add(1, Ordering::Release);
        Ok(())
    }

    pub(super) fn dequeue(&self) -> Option<BioReq> {
        if let Some(req) = self.queue.lock().pop_front() {
            self.num_reqs.fetch_sub(1, Ordering::Release);
            Some(req)
        } else {
            debug_assert_eq!(self.num_reqs.load(Ordering::Acquire), 0);
            None
        }
    }

    pub(super) fn num_reqs(&self) -> usize {
        self.num_reqs.load(Ordering::Acquire)
    }

    pub(super) fn is_empty(&self) -> bool {
        self.num_reqs() == 0
    }
}

pub struct BioReq {
    type_: BioType,
    addr: BlockId,
    nblocks: u32,
    bufs: Mutex<Vec<Buf>>,
    status: Mutex<BioStatus>,
    on_complete: Option<BioReqOnCompleteFn>,
    ext: Mutex<HashMap<TypeId, Box<dyn Any + Send + Sync>>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BioType {
    Read,
    Write,
    Sync,
}

pub(super) type BioResp = Result<()>;

pub(super) type BioReqOnCompleteFn = fn(&BioReq, &BioResp);

#[derive(Clone, Debug)]
enum BioStatus {
    Init,
    Submitted,
    Completed(BioResp),
}

impl BioReq {
    pub fn type_(&self) -> BioType {
        self.type_
    }

    pub fn addr(&self) -> BlockId {
        self.addr
    }

    pub fn access_bufs_with<F, R>(&self, mut f: F) -> R
    where
        F: FnMut(&[Buf]) -> R,
    {
        let bufs = self.bufs.lock();
        (f)(&bufs)
    }

    pub(super) fn access_mut_bufs_with<F, R>(&self, mut f: F) -> R
    where
        F: FnMut(&mut [Buf]) -> R,
    {
        let mut bufs = self.bufs.lock();
        (f)(&mut bufs)
    }

    pub(super) fn take_bufs(&self) -> Vec<Buf> {
        let mut bufs = self.bufs.lock();
        let mut ret_bufs = Vec::new();
        core::mem::swap(&mut *bufs, &mut ret_bufs);
        ret_bufs
    }

    pub fn nbufs(&self) -> usize {
        self.bufs.lock().len()
    }

    pub fn nblocks(&self) -> usize {
        self.nblocks as usize
    }

    pub fn ext(&self) -> MutexGuard<'_, HashMap<TypeId, Box<dyn Any + Send + Sync>>> {
        self.ext.lock()
    }

    pub(super) fn complete(&self, resp: BioResp) {
        let mut status = self.status.lock();
        match *status {
            BioStatus::Submitted => {
                if let Some(on_complete) = self.on_complete {
                    (on_complete)(self, &resp);
                }

                *status = BioStatus::Completed(resp);
            }
            _ => panic!("cannot complete before submitting or complete twice"),
        }
    }

    pub(super) fn submit(&self) {
        let mut status = self.status.lock();
        match *status {
            BioStatus::Init => *status = BioStatus::Submitted,
            _ => unreachable!(),
        }
    }
}

pub(super) struct BioReqBuilder {
    type_: BioType,
    addr: Option<BlockId>,
    bufs: Option<Vec<Buf>>,
    on_complete: Option<BioReqOnCompleteFn>,
    ext: Option<HashMap<TypeId, Box<dyn Any + Send + Sync>>>,
}

impl BioReqBuilder {
    pub(super) fn new(type_: BioType) -> Self {
        Self {
            type_,
            addr: None,
            bufs: None,
            on_complete: None,
            ext: None,
        }
    }

    pub(super) fn addr(mut self, addr: BlockId) -> Self {
        self.addr = Some(addr);
        self
    }

    pub(super) fn bufs(mut self, bufs: Vec<Buf>) -> Self {
        self.bufs = Some(bufs);
        self
    }

    pub(super) fn on_complete(mut self, on_complete: BioReqOnCompleteFn) -> Self {
        self.on_complete = Some(on_complete);
        self
    }

    pub(super) fn ext<T: Any + Send + Sync + Sized>(mut self, obj: T) -> Self {
        if self.ext.is_none() {
            self.ext = Some(HashMap::new());
        }
        let _ = self
            .ext
            .as_mut()
            .unwrap()
            .insert(TypeId::of::<T>(), Box::new(obj));
        self
    }

    pub(super) fn build(mut self) -> BioReq {
        let type_ = self.type_;
        if type_ == BioType::Sync {
            debug_assert!(
                self.addr.is_none(),
                "addr is only meaningful for a read or write",
            );
            debug_assert!(
                self.bufs.is_none(),
                "bufs is only meaningful for a read or write",
            );
        }

        let addr = self.addr.unwrap_or(0 as BlockId);

        let bufs = self.bufs.take().unwrap_or_default();
        let nblocks = {
            let nbytes = bufs
                .iter()
                .map(|buf| buf.as_slice().len())
                .fold(0_usize, |sum, len| sum.saturating_add(len));
            (nbytes / BLOCK_SIZE) as u32
        };

        let ext = self.ext.take().unwrap_or_default();
        let on_complete = self.on_complete.take();

        BioReq {
            type_,
            addr,
            nblocks,
            bufs: Mutex::new(bufs),
            status: Mutex::new(BioStatus::Init),
            on_complete,
            ext: Mutex::new(ext),
        }
    }
}
