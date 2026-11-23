#[derive(Debug)]
pub enum FuseError {
    BufferTooSmall,
    LengthOverflow,
    MalformedResponse,
    PageFault,
    RemoteError(i32),
    ResourceAlloc(ostd::Error),
}

pub type FuseResult<T> = Result<T, FuseError>;
