use alloc::boxed::Box;

use crate::FuseResult;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FuseCompletion {
    Complete(usize),
    ShortWrite { payload_len: usize, written: usize },
    MalformedResponse,
    RemoteError(i32),
}

impl FuseCompletion {
    pub fn payload_len(self) -> FuseResult<usize> {
        match self {
            Self::Complete(payload_len) | Self::ShortWrite { payload_len, .. } => Ok(payload_len),
            Self::MalformedResponse => Err(crate::FuseError::MalformedResponse),
            Self::RemoteError(error) => Err(crate::FuseError::RemoteError(error)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FuseStatus {
    Pending,
    Completed(FuseCompletion),
}

impl FuseStatus {
    pub fn is_pending(self) -> bool {
        self == Self::Pending
    }

    pub fn has_completed(self) -> Option<FuseCompletion> {
        match self {
            Self::Pending => None,
            Self::Completed(completion) => Some(completion),
        }
    }
}

pub type FuseCompleteFn = Box<dyn FnOnce(FuseCompletion) + Send>;
