#![short_vis_path::add(netlink)]

use crate::prelude::*;

pub(crate) enum ContinueRead<T, E = Error> {
    Parsed(T),
    SkippedInvalid(E),
}

pub(crate) enum ContinueAttrRead<T> {
    Parsed(T),
    SkippedUnsupported,
    SkippedInvalid(Error),
}

impl<T, E> ContinueRead<T, E> {
    pub(in netlink) fn map<F, U>(self, f: F) -> ContinueRead<U, E>
    where
        F: FnOnce(T) -> U,
    {
        match self {
            Self::Parsed(val) => ContinueRead::Parsed(f(val)),
            Self::SkippedInvalid(err) => ContinueRead::SkippedInvalid(err),
        }
    }

    pub(in netlink) fn map_err<F, U>(self, f: F) -> ContinueRead<T, U>
    where
        F: FnOnce(E) -> U,
    {
        match self {
            Self::Parsed(val) => ContinueRead::Parsed(val),
            Self::SkippedInvalid(err) => ContinueRead::SkippedInvalid(f(err)),
        }
    }
}
