mod c_types;
mod device;
mod driver;
mod file;
mod ioctl_defs;
mod keyboard;
mod manager;

use core::num::NonZeroU8;

pub(super) use driver::VtDriver;
pub(super) use manager::active_vt;

use crate::prelude::*;

const MAX_CONSOLES: usize = 63;

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct VtIndex(NonZeroU8);

impl VtIndex {
    const fn new(value: u8) -> Option<Self> {
        if value == 0 || value as usize > MAX_CONSOLES {
            None
        } else {
            Some(VtIndex(NonZeroU8::new(value).unwrap()))
        }
    }

    const fn get(self) -> u8 {
        self.0.get()
    }

    const fn to_zero_based(self) -> usize {
        (self.get() as usize) - 1
    }

    fn next_wrap(self) -> Self {
        let next = if (self.get() as usize) >= MAX_CONSOLES {
            1
        } else {
            self.get() + 1
        };

        Self(NonZeroU8::new(next).unwrap())
    }

    fn prev_wrap(self) -> Self {
        let prev = if self.get() == 1 {
            MAX_CONSOLES as u8
        } else {
            self.get() - 1
        };

        Self(NonZeroU8::new(prev).unwrap())
    }
}

impl TryFrom<i32> for VtIndex {
    type Error = Error;

    fn try_from(value: i32) -> Result<Self> {
        if let Ok(value) = u8::try_from(value)
            && let Some(index) = Self::new(value)
        {
            Ok(index)
        } else {
            Err(errno!(ENXIO, "the VT index is out of range"))
        }
    }
}

pub(super) fn init_in_first_process() -> Result<()> {
    manager::init_in_first_process()?;
    keyboard::init_in_first_process();
    device::init_in_first_process();
    Ok(())
}
