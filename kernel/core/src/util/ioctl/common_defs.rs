//! Common `ioctl` command definitions.
//!
//! This module defines `ioctl` commands that are widely supported across various
//! file and device types.

use crate::{ioc, util::ioctl::OutData};

pub(crate) type GetNumBytesToRead = ioc!(FIONREAD, 0x541B, OutData<i32>);
