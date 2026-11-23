use crate::arch::device::io_port::{IoPortReadAccess, IoPortWriteAccess, PortRead, PortWrite};
mod allocator;

use core::marker::PhantomData;

pub(super) use self::allocator::init;
use crate::{Error, prelude::*};

#[derive(Debug)]
pub struct IoPort<T, A> {
    port: u16,
    is_overlapping: bool,
    value_marker: PhantomData<T>,
    access_marker: PhantomData<A>,
}

impl<T, A> IoPort<T, A> {
    pub fn acquire(port: u16) -> Result<IoPort<T, A>> {
        allocator::IO_PORT_ALLOCATOR
            .get()
            .unwrap()
            .acquire(port, false)
            .ok_or(Error::AccessDenied)
    }

    pub fn acquire_overlapping(port: u16) -> Result<IoPort<T, A>> {
        allocator::IO_PORT_ALLOCATOR
            .get()
            .unwrap()
            .acquire(port, true)
            .ok_or(Error::AccessDenied)
    }

    pub const fn port(&self) -> u16 {
        self.port
    }

    pub const fn size(&self) -> u16 {
        size_of::<T>() as u16
    }

    pub(crate) const unsafe fn new(port: u16) -> Self {
        unsafe { Self::new_overlapping(port, false) }
    }

    const unsafe fn new_overlapping(port: u16, is_overlapping: bool) -> Self {
        Self {
            port,
            is_overlapping,
            value_marker: PhantomData,
            access_marker: PhantomData,
        }
    }
}

impl<T: PortRead, A: IoPortReadAccess> IoPort<T, A> {
    pub fn read(&self) -> T {
        unsafe { PortRead::read_from_port(self.port) }
    }
}

impl<T: PortWrite, A: IoPortWriteAccess> IoPort<T, A> {
    pub fn write(&self, value: T) {
        unsafe { PortWrite::write_to_port(self.port, value) }
    }
}

impl<T, A> Drop for IoPort<T, A> {
    fn drop(&mut self) {
        let port = self.port as usize;
        let range = if !self.is_overlapping {
            port..port + size_of::<T>()
        } else {
            port..port + 1
        };

        unsafe { allocator::IO_PORT_ALLOCATOR.get().unwrap().recycle(range) };
    }
}

macro_rules! reserve_io_port_range {
    ($range:expr) => {
        crate::const_assert!(
            $range.start < $range.end,
            "I/O port range must be valid (start < end)"
        );

        const _: () = {
            #[used]
            #[unsafe(link_section = ".sensitive_io_ports")]
            static _RANGE: crate::io::RawIoPortRange = crate::io::RawIoPortRange {
                begin: $range.start,
                end: $range.end,
            };
        };
    };
}

macro_rules! sensitive_io_port {
    (unsafe { $(
        $(#[$meta:meta])*
        $vis:vis static $name:ident: IoPort<$size:ty, $access:ty> = IoPort::new($port:expr);
    )* }) => {
        $(
            $(#[$meta])*
            $vis static $name: IoPort<$size, $access> = {
                #[used]
                #[unsafe(link_section = ".sensitive_io_ports")]
                static _RESERVED_IO_PORT_RANGE: crate::io::RawIoPortRange = crate::io::RawIoPortRange {
                    begin: $name.port(),
                    end: $name.port() + $name.size(),
                };

            	unsafe {
                     IoPort::new($port)
            	}
            };
        )*
    };
}

pub(crate) use reserve_io_port_range;
pub(crate) use sensitive_io_port;

#[doc(hidden)]
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct RawIoPortRange {
    pub(crate) begin: u16,
    pub(crate) end: u16,
}
