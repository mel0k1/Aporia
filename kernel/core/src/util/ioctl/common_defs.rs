use crate::{ioc, util::ioctl::OutData};

pub(crate) type GetNumBytesToRead = ioc!(FIONREAD, 0x541B, OutData<i32>);
