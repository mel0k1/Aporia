use aster_fuse::{FuseCompletion, FuseError, FuseOperation, FuseStatus, ReplyHeader};
use io_util::{IoError, batch::IoCompletion};
use ostd::{
    mm::io::util::HasVmReaderWriter,
    sync::{LocalIrqDisabled, SpinLock, WaitQueue},
};
use smallvec::SmallVec;

use crate::device::filesystem::pool::FuseReplyBuf;

pub(super) struct ReplyBufs {
    header: Option<FuseReplyBuf>,
    payload: SmallVec<[FuseReplyBuf; 1]>,
}

impl ReplyBufs {
    pub(super) fn new(header: Option<FuseReplyBuf>) -> Self {
        Self {
            header,
            payload: SmallVec::new(),
        }
    }

    pub(super) fn push_payload(&mut self, payload: FuseReplyBuf) {
        debug_assert!(self.header.is_some());
        self.payload.push(payload);
    }

    pub(super) fn is_empty(&self) -> bool {
        self.header.is_none()
    }

    pub(super) fn reply_header_buf(&self) -> Result<&FuseReplyBuf, FuseError> {
        self.header.as_ref().ok_or(FuseError::MalformedResponse)
    }

    pub(super) fn payload_bufs(&self) -> &[FuseReplyBuf] {
        &self.payload
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = &FuseReplyBuf> {
        self.header.iter().chain(self.payload.iter())
    }

    fn single_payload(&self) -> Result<Option<&FuseReplyBuf>, FuseError> {
        match self.payload_bufs() {
            [] => Ok(None),
            [payload] => Ok(Some(payload)),
            [..] => Err(FuseError::MalformedResponse),
        }
    }
}

#[must_use]
pub struct FuseWaiter {
    reply_bufs: ReplyBufs,
    status: SpinLock<FuseStatus, LocalIrqDisabled>,
    wait_queue: WaitQueue,
}

impl FuseWaiter {
    pub(super) fn new(reply_bufs: ReplyBufs) -> Self {
        Self {
            reply_bufs,
            status: SpinLock::new(FuseStatus::Pending),
            wait_queue: WaitQueue::new(),
        }
    }

    pub(super) fn parse_reply<Op: FuseOperation>(
        &self,
        payload_len: usize,
    ) -> Result<Op::Output, FuseError> {
        let mut reader = if let Some(payload_buf) = self.reply_bufs.single_payload()? {
            payload_buf.reader().unwrap()
        } else {
            let header_buf = self.reply_bufs.reply_header_buf()?;
            let mut reader = header_buf.reader().unwrap();
            reader.skip(size_of::<ReplyHeader>());
            reader
        };

        Op::parse_reply(payload_len, &mut reader)
    }

    pub(super) fn wait(&self) -> FuseCompletion {
        self.wait_queue.wait_until(|| {
            let status = *self.status.lock();
            status.has_completed()
        })
    }

    pub(super) fn wake_completed(&self, completion: FuseCompletion) {
        let should_wake = {
            let mut current_status = self.status.lock();
            if !current_status.is_pending() {
                false
            } else {
                *current_status = FuseStatus::Completed(completion);
                true
            }
        };

        if should_wake {
            self.wait_queue.wake_all();
        }
    }

    pub(super) fn reply_bufs(&self) -> &ReplyBufs {
        &self.reply_bufs
    }
}

impl IoCompletion for FuseWaiter {
    fn wait(&self) -> Result<(), IoError> {
        match self.wait() {
            FuseCompletion::Complete(_) | FuseCompletion::ShortWrite { .. } => Ok(()),
            FuseCompletion::MalformedResponse | FuseCompletion::RemoteError(_) => {
                Err(IoError::Failed)
            }
        }
    }
}
