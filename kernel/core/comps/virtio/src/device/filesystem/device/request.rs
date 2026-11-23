use alloc::sync::Arc;

use aster_fuse::{
    FuseCompleteFn, FuseCompletion, FuseNodeId, FuseUnique, ReplyExpectation, ReplyHeader,
    WriteReply,
};
use ostd::mm::io::util::HasVmReaderWriter;
use smallvec::SmallVec;

use super::waiter::{FuseWaiter, ReplyBufs};
use crate::device::filesystem::pool::FuseRequestBuf;

pub(super) type RequestBufs = SmallVec<[FuseRequestBuf; 2]>;

pub(super) struct FuseRequest {
    unique: FuseUnique,
    nodeid: FuseNodeId,
    reply_expectation: ReplyExpectation,
    request_bufs: RequestBufs,
    waiter: Arc<FuseWaiter>,
    complete_fn: Option<FuseCompleteFn>,
}

impl FuseRequest {
    pub(super) fn new(
        unique: FuseUnique,
        nodeid: FuseNodeId,
        reply_expectation: ReplyExpectation,
        request_bufs: RequestBufs,
        reply_bufs: ReplyBufs,
        complete_fn: Option<FuseCompleteFn>,
    ) -> Self {
        Self {
            unique,
            nodeid,
            reply_expectation,
            request_bufs,
            waiter: Arc::new(FuseWaiter::new(reply_bufs)),
            complete_fn,
        }
    }

    pub(super) fn finish_reply(mut self, reply_len: usize) {
        let completion = self.parse_reply_header(reply_len);
        if let Some(complete_fn) = self.complete_fn.take() {
            complete_fn(completion);
        }

        self.waiter.wake_completed(completion);
    }

    pub(super) fn nodeid(&self) -> FuseNodeId {
        self.nodeid
    }

    pub(super) fn request_bufs(&self) -> &[FuseRequestBuf] {
        self.request_bufs.as_slice()
    }

    pub(super) fn num_dma_bufs(&self) -> usize {
        self.request_bufs.len() + self.waiter.reply_bufs().iter().count()
    }

    pub(super) fn waiter(&self) -> &Arc<FuseWaiter> {
        &self.waiter
    }

    fn parse_reply_header(&self, reply_len: usize) -> FuseCompletion {
        let reply_bufs = self.waiter.reply_bufs();
        if reply_bufs.is_empty() {
            return FuseCompletion::Complete(0);
        }
        if reply_len < size_of::<ReplyHeader>() {
            return FuseCompletion::MalformedResponse;
        }
        for reply_buf in reply_bufs.iter() {
            reply_buf.sync_from_device().unwrap();
        }

        let Ok(reply_header_buf) = reply_bufs.reply_header_buf() else {
            return FuseCompletion::MalformedResponse;
        };

        let mut reader = reply_header_buf.reader().unwrap();
        let Ok(reply_header) = reader.read_val::<ReplyHeader>() else {
            return FuseCompletion::MalformedResponse;
        };

        if reply_header.unique() != self.unique {
            return FuseCompletion::MalformedResponse;
        }

        if !self.is_reply_shape_valid(&reply_header, reply_len) {
            return FuseCompletion::MalformedResponse;
        }

        if reply_header.error() != 0 {
            return FuseCompletion::RemoteError(reply_header.error());
        }

        let payload_len = reply_header.len() as usize - size_of::<ReplyHeader>();

        match self.reply_expectation {
            ReplyExpectation::WritePayload { requested_size } => {
                let Some(payload_buf) = reply_bufs.payload_bufs().first() else {
                    return FuseCompletion::MalformedResponse;
                };
                let mut reader = payload_buf.reader().unwrap();
                let Ok(write_reply) = reader.read_val::<WriteReply>() else {
                    return FuseCompletion::MalformedResponse;
                };
                let written = write_reply.size();
                if written > requested_size {
                    FuseCompletion::MalformedResponse
                } else if written < requested_size {
                    FuseCompletion::ShortWrite {
                        payload_len,
                        written,
                    }
                } else {
                    FuseCompletion::Complete(payload_len)
                }
            }
            _ => FuseCompletion::Complete(payload_len),
        }
    }

    fn is_reply_shape_valid(&self, reply_header: &ReplyHeader, reply_len: usize) -> bool {
        let reply_header_len = reply_header.len() as usize;

        if reply_header_len != reply_len {
            return false;
        }

        let declared_payload_len = reply_header_len - size_of::<ReplyHeader>();

        if reply_header.error() != 0 {
            return declared_payload_len == 0;
        }

        match self.reply_expectation {
            ReplyExpectation::None => false,
            ReplyExpectation::HeaderOnly => declared_payload_len == 0,
            ReplyExpectation::FixedPayload(expected_payload_len) => {
                declared_payload_len == expected_payload_len.get()
            }
            ReplyExpectation::VariablePayload(max_payload_len) => {
                declared_payload_len <= max_payload_len.get()
            }
            ReplyExpectation::WritePayload { .. } => {
                declared_payload_len == size_of::<WriteReply>()
            }
        }
    }
}
