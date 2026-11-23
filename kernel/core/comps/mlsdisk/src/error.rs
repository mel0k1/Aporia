use core::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Errno {
    TxAborted,
    NotFound,
    InvalidArgs,
    OutOfMemory,
    OutOfDisk,
    IoFailed,
    PermissionDenied,
    Unsupported,
    OsSpecUnknown,
    EncryptFailed,
    DecryptFailed,
    MacMismatched,
    NotBlockSizeAligned,
    TryLockFailed,
}

#[derive(Clone, Debug)]
pub struct Error {
    errno: Errno,
    msg: Option<&'static str>,
}

impl Error {
    pub const fn new(errno: Errno) -> Self {
        Error { errno, msg: None }
    }

    pub const fn with_msg(errno: Errno, msg: &'static str) -> Self {
        Error {
            errno,
            msg: Some(msg),
        }
    }

    pub fn errno(&self) -> Errno {
        self.errno
    }
}

impl From<Errno> for Error {
    fn from(errno: Errno) -> Self {
        Error::new(errno)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl fmt::Display for Errno {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[macro_export]
macro_rules! return_errno {
    ($errno: expr) => {
        return core::result::Result::Err($crate::Error::new($errno))
    };
}

#[macro_export]
macro_rules! return_errno_with_msg {
    ($errno: expr, $msg: expr) => {
        return core::result::Result::Err($crate::Error::with_msg($errno, $msg))
    };
}
