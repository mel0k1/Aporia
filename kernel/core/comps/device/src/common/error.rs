use core::fmt;

#[derive(Debug)]
pub enum Error {
    AlreadyAdded,
    NotAdded,
    ParentNotAdded,
    HasChildren,
    NameConflict,
    NotFound,
    InvalidName,
    NoDriver,
    AlreadyBound,
    NotBound,
    ProbeFailed,
    DriverUnregistered,
    Attribute,
    NoDevNum,
    Format,
    InvalidValue,
    ResourceUnavailable,
    MemoryBudgetExhausted,
    Hook,
    SysTree(aster_systree::Error),
}

pub type Result<T, E = Error> = core::result::Result<T, E>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::AlreadyAdded => write!(f, "the object has already been added"),
            Error::NotAdded => write!(f, "the device has not been added"),
            Error::ParentNotAdded => write!(f, "the parent device has not been added"),
            Error::HasChildren => write!(f, "the device still has child devices"),
            Error::NameConflict => write!(f, "a sibling with the same name exists"),
            Error::NotFound => write!(f, "the object does not exist"),
            Error::InvalidName => write!(f, "the name is invalid"),
            Error::NoDriver => write!(f, "no driver accepted the device"),
            Error::AlreadyBound => write!(f, "the device is already bound to a driver"),
            Error::NotBound => write!(f, "the device is not bound to a driver"),
            Error::ProbeFailed => write!(f, "the driver rejected the device"),
            Error::DriverUnregistered => write!(f, "the driver has been unregistered"),
            Error::NoDevNum => write!(f, "the device has no device number"),
            Error::Format => write!(f, "formatting an attribute value failed"),
            Error::Attribute => write!(f, "attribute operation failed"),
            Error::InvalidValue => write!(f, "invalid attribute value"),
            Error::ResourceUnavailable => write!(f, "resource unavailable"),
            Error::MemoryBudgetExhausted => write!(f, "memory budget is exhausted"),
            Error::Hook => write!(f, "kernel hook failed"),
            Error::SysTree(e) => write!(f, "systree error: {}", e),
        }
    }
}

impl From<aster_systree::Error> for Error {
    fn from(e: aster_systree::Error) -> Self {
        match e {
            aster_systree::Error::AlreadyExists => Error::NameConflict,
            aster_systree::Error::NotFound => Error::NotFound,
            aster_systree::Error::ResourceUnavailable => Error::ResourceUnavailable,
            aster_systree::Error::MemoryBudgetExhausted => Error::MemoryBudgetExhausted,
            other => Error::SysTree(other),
        }
    }
}

impl From<fmt::Error> for Error {
    fn from(_: fmt::Error) -> Self {
        Error::Format
    }
}

impl From<Error> for aster_systree::Error {
    fn from(e: Error) -> Self {
        match e {
            Error::NotFound => aster_systree::Error::NotFound,
            Error::NameConflict => aster_systree::Error::AlreadyExists,
            Error::InvalidValue | Error::InvalidName => aster_systree::Error::InvalidOperation,
            Error::ResourceUnavailable => aster_systree::Error::ResourceUnavailable,
            Error::MemoryBudgetExhausted => aster_systree::Error::MemoryBudgetExhausted,
            Error::NotAdded => aster_systree::Error::IsDead,
            Error::SysTree(inner) => inner,
            _ => aster_systree::Error::AttributeError,
        }
    }
}
