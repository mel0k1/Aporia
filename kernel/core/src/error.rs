use core::panic::Location;

use int_to_c_enum::TryFromInt;

/// Error number.
#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, TryFromInt)]
pub enum Errno {
    EPERM = 1,
    ENOENT = 2,
    ESRCH = 3,
    EINTR = 4,
    EIO = 5,
    ENXIO = 6,
    E2BIG = 7,
    ENOEXEC = 8,
    EBADF = 9,
    ECHILD = 10,
    EAGAIN = 11,
    ENOMEM = 12,
    EACCES = 13,
    EFAULT = 14,
    ENOTBLK = 15,
    EBUSY = 16,
    EEXIST = 17,
    EXDEV = 18,
    ENODEV = 19,
    ENOTDIR = 20,
    EISDIR = 21,
    EINVAL = 22,
    ENFILE = 23,
    EMFILE = 24,
    ENOTTY = 25,
    ETXTBSY = 26,
    EFBIG = 27,
    ENOSPC = 28,
    ESPIPE = 29,
    EROFS = 30,
    EMLINK = 31,
    EPIPE = 32,
    EDOM = 33,
    ERANGE = 34,

    EDEADLK = 35,
    ENAMETOOLONG = 36,
    ENOLCK = 37,

    ENOSYS = 38,
    ENOTEMPTY = 39,
    ELOOP = 40,

    ENOMSG = 42,
    EIDRM = 43,
    ECHRNG = 44,
    EL2NSYNC = 45,
    EL3HLT = 46,
    EL3RST = 47,
    ELNRNG = 48,
    EUNATCH = 49,
    ENOCSI = 50,
    EL2HLT = 51,
    EBADE = 52,
    EBADR = 53,
    EXFULL = 54,
    ENOANO = 55,
    EBADRQC = 56,
    EBADSLT = 57,

    EBFONT = 59,
    ENOSTR = 60,
    ENODATA = 61,
    ETIME = 62,
    ENOSR = 63,
    ENONET = 64,
    ENOPKG = 65,
    EREMOTE = 66,
    ENOLINK = 67,
    EADV = 68,
    ESRMNT = 69,
    ECOMM = 70,
    EPROTO = 71,
    EMULTIHOP = 72,
    EDOTDOT = 73,
    EBADMSG = 74,
    EOVERFLOW = 75,
    ENOTUNIQ = 76,
    EBADFD = 77,
    EREMCHG = 78,
    ELIBACC = 79,
    ELIBBAD = 80,
    ELIBSCN = 81,
    ELIBMAX = 82,
    ELIBEXEC = 83,
    EILSEQ = 84,
    ERESTART = 85,
    ESTRPIPE = 86,
    EUSERS = 87,
    ENOTSOCK = 88,
    EDESTADDRREQ = 89,
    EMSGSIZE = 90,
    EPROTOTYPE = 91,
    ENOPROTOOPT = 92,
    EPROTONOSUPPORT = 93,
    ESOCKTNOSUPPORT = 94,
    EOPNOTSUPP = 95,
    EPFNOSUPPORT = 96,
    EAFNOSUPPORT = 97,
    EADDRINUSE = 98,
    EADDRNOTAVAIL = 99,
    ENETDOWN = 100,
    ENETUNREACH = 101,
    ENETRESET = 102,
    ECONNABORTED = 103,
    ECONNRESET = 104,
    ENOBUFS = 105,
    EISCONN = 106,
    ENOTCONN = 107,
    ESHUTDOWN = 108,
    ETOOMANYREFS = 109,
    ETIMEDOUT = 110,
    ECONNREFUSED = 111,
    EHOSTDOWN = 112,
    EHOSTUNREACH = 113,
    EALREADY = 114,
    EINPROGRESS = 115,
    ESTALE = 116,
    EUCLEAN = 117,
    ENOTNAM = 118,
    ENAVAIL = 119,
    EISNAM = 120,
    EREMOTEIO = 121,
    EDQUOT = 122,
    ENOMEDIUM = 123,
    EMEDIUMTYPE = 124,
    ECANCELED = 125,
    ENOKEY = 126,
    EKEYEXPIRED = 127,
    EKEYREVOKED = 128,
    EKEYREJECTED = 129,

    EOWNERDEAD = 130,
    ENOTRECOVERABLE = 131,

    ERFKILL = 132,

    EHWPOISON = 133,

    ERESTARTSYS = 512,
}

/// An error with an error number and a source location.
///
/// The location identifies where this error was constructed,
/// including where an external error was converted into this type.
/// Propagating an existing error preserves its original location.
#[derive(Clone, Copy, Debug)]
pub struct Error {
    errno: Errno,
    location: &'static Location<'static>,
}

impl Error {
    /// Creates an error with the given error number and the caller's source location.
    ///
    /// Use [`errno!`](crate::errno) when the error number is a known variant.
    /// Call this method directly when the error number is computed at runtime.
    #[track_caller]
    pub const fn new(errno: Errno) -> Self {
        Error {
            errno,
            location: Location::caller(),
        }
    }

    /// Returns the location where this error was constructed.
    ///
    /// Propagating the error preserves this location.
    pub const fn location(&self) -> &'static Location<'static> {
        self.location
    }

    /// Returns the error number.
    pub const fn error(&self) -> Errno {
        self.errno
    }
}

/// Constructs a kernel error at the call site from an `Errno` variant name.
///
/// The optional message documents the error in the source; it is neither
/// evaluated nor stored in the error.
#[macro_export]
macro_rules! errno {
    ($errno:ident $(, $message:expr)? $(,)?) => {
        $crate::error::Error::new($crate::error::Errno::$errno)
    };
}

impl From<Errno> for Error {
    #[track_caller]
    fn from(errno: Errno) -> Self {
        Error::new(errno)
    }
}

impl AsRef<Error> for Error {
    fn as_ref(&self) -> &Error {
        self
    }
}

impl From<ostd::Error> for Error {
    #[track_caller]
    fn from(ostd_error: ostd::Error) -> Self {
        match ostd_error {
            ostd::Error::AccessDenied => errno!(EFAULT),
            ostd::Error::NoMemory => errno!(ENOMEM),
            ostd::Error::InvalidArgs => errno!(EINVAL),
            ostd::Error::IoError => errno!(EIO),
            ostd::Error::NotEnoughResources => errno!(EBUSY),
            ostd::Error::PageFault => errno!(EFAULT),
            ostd::Error::Overflow => errno!(EOVERFLOW),
        }
    }
}

impl From<(ostd::Error, usize)> for Error {
    #[track_caller]
    fn from(ostd_error: (ostd::Error, usize)) -> Self {
        Error::from(ostd_error.0)
    }
}

impl From<(Error, usize)> for Error {
    fn from(err: (Error, usize)) -> Self {
        err.0
    }
}

impl From<aster_block::bio::BioEnqueueError> for Error {
    #[track_caller]
    fn from(error: aster_block::bio::BioEnqueueError) -> Self {
        match error {
            aster_block::bio::BioEnqueueError::IsFull => {
                errno!(EBUSY, "The request queue is full")
            }
            aster_block::bio::BioEnqueueError::Refused => {
                errno!(EBUSY, "Refuse to enqueue the bio")
            }
            aster_block::bio::BioEnqueueError::TooBig => {
                errno!(EINVAL, "Bio is too big")
            }
        }
    }
}

impl From<aster_block::bio::BioStatus> for Error {
    #[track_caller]
    fn from(err_status: aster_block::bio::BioStatus) -> Self {
        match err_status {
            aster_block::bio::BioStatus::NotSupported => {
                errno!(EOPNOTSUPP, "I/O operation is not supported")
            }
            aster_block::bio::BioStatus::NoSpace => {
                errno!(ENOSPC, "Insufficient space on device")
            }
            aster_block::bio::BioStatus::IoError => {
                errno!(EIO, "I/O operation fails")
            }
            status => panic!("Can not convert the status: {:?} to an error", status),
        }
    }
}

impl From<io_util::IoError> for Error {
    #[track_caller]
    fn from(error: io_util::IoError) -> Self {
        match error {
            io_util::IoError::Unsupported => {
                errno!(EOPNOTSUPP, "I/O operation is not supported")
            }
            io_util::IoError::OutOfSpace => {
                errno!(ENOSPC, "Insufficient space on device")
            }
            io_util::IoError::Failed => errno!(EIO, "I/O operation fails"),
        }
    }
}

impl From<core::num::TryFromIntError> for Error {
    #[track_caller]
    fn from(_: core::num::TryFromIntError) -> Self {
        errno!(EINVAL, "Invalid integer")
    }
}

impl From<core::str::Utf8Error> for Error {
    #[track_caller]
    fn from(_: core::str::Utf8Error) -> Self {
        errno!(EINVAL, "Invalid utf-8 string")
    }
}

impl From<alloc::string::FromUtf8Error> for Error {
    #[track_caller]
    fn from(_: alloc::string::FromUtf8Error) -> Self {
        errno!(EINVAL, "Invalid utf-8 string")
    }
}

impl From<core::ffi::FromBytesUntilNulError> for Error {
    #[track_caller]
    fn from(_: core::ffi::FromBytesUntilNulError) -> Self {
        errno!(E2BIG, "Cannot find null in cstring")
    }
}

impl From<core::ffi::FromBytesWithNulError> for Error {
    #[track_caller]
    fn from(_: core::ffi::FromBytesWithNulError) -> Self {
        errno!(E2BIG, "Cannot find null in cstring")
    }
}

impl From<cpio_decoder::error::Error> for Error {
    #[track_caller]
    fn from(cpio_error: cpio_decoder::error::Error) -> Self {
        match cpio_error {
            cpio_decoder::error::Error::MagicError => {
                errno!(EINVAL, "CPIO invalid magic number")
            }
            cpio_decoder::error::Error::Utf8Error => {
                errno!(EINVAL, "CPIO invalid utf-8 string")
            }
            cpio_decoder::error::Error::ParseIntError => {
                errno!(EINVAL, "CPIO parse int error")
            }
            cpio_decoder::error::Error::FileTypeError => {
                errno!(EINVAL, "CPIO invalid file type")
            }
            cpio_decoder::error::Error::FileNameError => {
                errno!(EINVAL, "CPIO invalid file name")
            }
            cpio_decoder::error::Error::BufferShortError => {
                errno!(EINVAL, "CPIO buffer is too short")
            }
            cpio_decoder::error::Error::IoError => {
                errno!(EIO, "CPIO buffer I/O error")
            }
        }
    }
}

impl From<Error> for ostd::Error {
    fn from(error: Error) -> Self {
        match error.errno {
            Errno::EACCES => ostd::Error::AccessDenied,
            Errno::EIO => ostd::Error::IoError,
            Errno::ENOMEM => ostd::Error::NoMemory,
            Errno::EFAULT => ostd::Error::PageFault,
            Errno::EINVAL => ostd::Error::InvalidArgs,
            Errno::EBUSY => ostd::Error::NotEnoughResources,
            _ => ostd::Error::InvalidArgs,
        }
    }
}

impl From<alloc::ffi::NulError> for Error {
    #[track_caller]
    fn from(_: alloc::ffi::NulError) -> Self {
        errno!(E2BIG, "Cannot find null in cstring")
    }
}

impl From<int_to_c_enum::TryFromIntError> for Error {
    #[track_caller]
    fn from(_: int_to_c_enum::TryFromIntError) -> Self {
        errno!(EINVAL, "Invalid enum value")
    }
}

impl From<aster_device::common::Error> for Error {
    #[track_caller]
    fn from(err: aster_device::common::Error) -> Self {
        use aster_device::common::Error::*;
        match err {
            AlreadyAdded | AlreadyBound => errno!(EBUSY),
            NotAdded | NotBound | ParentNotAdded | DriverUnregistered | NoDevNum => {
                errno!(ENODEV)
            }
            HasChildren => errno!(ENOTEMPTY),
            NameConflict => errno!(EEXIST),
            NotFound | NoDriver => errno!(ENOENT),
            InvalidName | InvalidValue => errno!(EINVAL),
            ProbeFailed | Attribute | Format | Hook => errno!(EIO),
            ResourceUnavailable => errno!(ENOSPC),
            MemoryBudgetExhausted => errno!(ENOMEM),
            SysTree(inner) => inner.into(),
        }
    }
}

impl From<aster_systree::Error> for Error {
    #[track_caller]
    fn from(err: aster_systree::Error) -> Self {
        use aster_systree::Error::*;
        match err {
            NotFound => errno!(ENOENT),
            InvalidOperation | InvalidName => errno!(EINVAL),
            ResourceUnavailable => errno!(EBUSY),
            MemoryBudgetExhausted => errno!(ENOMEM),
            AttributeError => errno!(EIO),
            PermissionDenied => errno!(EACCES),
            InternalError(_msg) => errno!(EIO, _msg),
            AlreadyExists => errno!(EEXIST),
            Overflow => errno!(EOVERFLOW),
            PageFault => errno!(EFAULT),
            IsDead => errno!(ENODEV),
        }
    }
}

impl From<aster_util::printer::VmPrinterError> for Error {
    #[track_caller]
    fn from(value: aster_util::printer::VmPrinterError) -> Self {
        match value {
            aster_util::printer::VmPrinterError::PageFault => {
                errno!(EFAULT, "Page fault occurred during VmPrinter write")
            }
        }
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::*;

    use super::*;

    #[ktest]
    fn construction_location() {
        let line = line!() + 1;
        let error = Error::new(Errno::EINVAL);
        assert_eq!(error.location().file(), file!());
        assert_eq!(error.location().line(), line);

        let line = line!() + 1;
        let error = errno!(EINVAL, "invalid argument");
        assert_eq!(error.error(), Errno::EINVAL);
        assert_eq!(error.location().line(), line);

        let line = line!() + 1;
        const ERROR: Error = errno!(EINVAL,);
        assert_eq!(ERROR.error(), Errno::EINVAL);
        assert_eq!(ERROR.location().line(), line);

        let line = line!() + 2;
        fn fail() -> Result<(), Error> {
            Err(errno!(EINVAL))
        }
        assert_eq!(fail().unwrap_err().location().line(), line);

        let line = line!() + 2;
        fn fail_with_message() -> Result<(), Error> {
            Err(errno!(EINVAL, "invalid argument"))
        }
        assert_eq!(fail_with_message().unwrap_err().location().line(), line);
    }

    #[ktest]
    fn messages_are_not_evaluated() {
        const ERROR: Error = errno!(EINVAL, panic!("message must not be evaluated"),);
        assert_eq!(ERROR.error(), Errno::EINVAL);

        fn fail() -> Result<(), Error> {
            Err(errno!(EIO, panic!("message must not be evaluated"),))
        }
        assert_eq!(fail().unwrap_err().error(), Errno::EIO);
    }

    #[ktest]
    fn conversion_location() {
        let line = line!() + 1;
        let error = Error::from(Errno::EINVAL);
        assert_eq!(error.location().line(), line);

        let line = line!() + 1;
        let error: Error = ostd::Error::InvalidArgs.into();
        assert_eq!(error.location().line(), line);

        let line = line!() + 1;
        let error = Error::from((ostd::Error::PageFault, 0));
        assert_eq!(error.location().line(), line);

        let line = line!() + 2;
        fn convert() -> Result<(), Error> {
            Err(ostd::Error::InvalidArgs)?;
            Ok(())
        }
        let error = convert().unwrap_err();
        assert_eq!(error.location().file(), file!());
        assert_eq!(error.location().line(), line);
    }

    #[ktest]
    fn propagation_preserves_location() {
        fn propagate(error: Error) -> Result<(), Error> {
            Err(error)?;
            Ok(())
        }

        let original = errno!(EINVAL);
        let propagated = propagate(original).unwrap_err();
        assert_eq!(propagated.location(), original.location());

        let unpacked = Error::from((original, 0));
        assert_eq!(unpacked.location(), original.location());
    }
}
