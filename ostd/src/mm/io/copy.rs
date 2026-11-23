use super::{Fallible, Infallible, Io};
use crate::arch::{
    io::io_mem::{
        copy_from_mmio, copy_from_mmio_fallible, copy_to_mmio, copy_to_mmio_fallible, memset_mmio,
    },
    mm::{__memcpy_fallible, __memset_fallible},
};

pub(crate) unsafe fn memcpy<Dst: Memcpy<Src>, Src>(
    dst: *mut u8,
    src: *const u8,
    len: usize,
) -> Dst::Result {
    unsafe { Dst::memcpy(dst, src, len) }
}

pub(crate) unsafe fn memset<Dst: Memset>(dst: *mut u8, value: u8, len: usize) -> Dst::Result {
    unsafe { Dst::memset(dst, value, len) }
}

pub(crate) unsafe trait Memcpy<Src> {
    type Result;

    unsafe fn memcpy(dst: *mut u8, src: *const u8, len: usize) -> Self::Result;
}

pub(crate) unsafe trait Memset {
    type Result;

    unsafe fn memset(dst: *mut u8, value: u8, len: usize) -> Self::Result;
}

unsafe impl Memcpy<Infallible> for Infallible {
    type Result = ();

    unsafe fn memcpy(dst: *mut u8, src: *const u8, len: usize) {
        unsafe { core::intrinsics::volatile_copy_memory(dst, src, len) };
    }
}

unsafe impl Memcpy<Fallible> for Infallible {
    type Result = usize;

    unsafe fn memcpy(dst: *mut u8, src: *const u8, len: usize) -> usize {
        let failed_bytes = unsafe { __memcpy_fallible(dst, src, len) };
        len - failed_bytes
    }
}

unsafe impl Memcpy<Io> for Infallible {
    type Result = ();

    unsafe fn memcpy(dst: *mut u8, src: *const u8, len: usize) {
        unsafe { copy_from_mmio(dst, src, len) };
    }
}

unsafe impl Memcpy<Infallible> for Fallible {
    type Result = usize;

    unsafe fn memcpy(dst: *mut u8, src: *const u8, len: usize) -> usize {
        let failed_bytes = unsafe { __memcpy_fallible(dst, src, len) };
        len - failed_bytes
    }
}

unsafe impl Memcpy<Fallible> for Fallible {
    type Result = usize;

    unsafe fn memcpy(dst: *mut u8, src: *const u8, len: usize) -> usize {
        let failed_bytes = unsafe { __memcpy_fallible(dst, src, len) };
        len - failed_bytes
    }
}

unsafe impl Memcpy<Io> for Fallible {
    type Result = usize;

    unsafe fn memcpy(dst: *mut u8, src: *const u8, len: usize) -> usize {
        unsafe { copy_from_mmio_fallible(dst, src, len) }
    }
}

unsafe impl Memcpy<Infallible> for Io {
    type Result = ();

    unsafe fn memcpy(dst: *mut u8, src: *const u8, len: usize) {
        unsafe { copy_to_mmio(src, dst, len) };
    }
}

unsafe impl Memcpy<Fallible> for Io {
    type Result = usize;

    unsafe fn memcpy(dst: *mut u8, src: *const u8, len: usize) -> usize {
        unsafe { copy_to_mmio_fallible(src, dst, len) }
    }
}

unsafe impl Memset for Infallible {
    type Result = ();

    unsafe fn memset(dst: *mut u8, value: u8, len: usize) {
        unsafe { core::intrinsics::volatile_set_memory(dst, value, len) };
    }
}

unsafe impl Memset for Fallible {
    type Result = usize;

    unsafe fn memset(dst: *mut u8, value: u8, len: usize) -> usize {
        let failed_bytes = unsafe { __memset_fallible(dst, value, len) };
        len - failed_bytes
    }
}

unsafe impl Memset for Io {
    type Result = ();

    unsafe fn memset(dst: *mut u8, value: u8, len: usize) {
        unsafe { memset_mmio(dst, value, len) };
    }
}
