use core::arch::asm;

use super::cvm;
use crate::{
    arch::{if_tdx_enabled, mm::__memcpy_fallible},
    mm::PodOnce,
};

pub(crate) unsafe fn read_once<T: PodOnce>(ptr: *const T) -> T {
    debug_assert!(ptr.is_aligned());
    let mut val: u64 = 0;

    unsafe {
        match size_of::<T>() {
            1 => {
                asm!("mov {0:l}, [{1}]", out(reg) val, in(reg) ptr, options(nostack, readonly, preserves_flags))
            }
            2 => {
                asm!("mov {0:x}, [{1}]", out(reg) val, in(reg) ptr, options(nostack, readonly, preserves_flags))
            }
            4 => {
                asm!("mov {0:e}, [{1}]", out(reg) val, in(reg) ptr, options(nostack, readonly, preserves_flags))
            }
            8 => {
                asm!("mov {0}, [{1}]", out(reg) val, in(reg) ptr, options(nostack, readonly, preserves_flags))
            }
            _ => core::hint::unreachable_unchecked(),
        }
        core::ptr::read((&val as *const u64).cast::<T>())
    }
}

pub(crate) unsafe fn write_once<T: PodOnce>(ptr: *mut T, val: T) {
    debug_assert!(ptr.is_aligned());
    let mut tmp: u64 = 0;

    unsafe {
        core::ptr::write((&mut tmp as *mut u64).cast::<T>(), val);

        match size_of::<T>() {
            1 => {
                asm!("mov [{0}], {1:l}", in(reg) ptr, in(reg) tmp, options(nostack, preserves_flags))
            }
            2 => {
                asm!("mov [{0}], {1:x}", in(reg) ptr, in(reg) tmp, options(nostack, preserves_flags))
            }
            4 => {
                asm!("mov [{0}], {1:e}", in(reg) ptr, in(reg) tmp, options(nostack, preserves_flags))
            }
            8 => {
                asm!("mov [{0}], {1}", in(reg) ptr, in(reg) tmp, options(nostack, preserves_flags))
            }
            _ => core::hint::unreachable_unchecked(),
        }
    }
}

macro_rules! do_copy_mmio_impl {
    (dst: $dst:ident, src: $src:ident, count: $count:ident) => {{
        if $count == 0 { return; }

        unsafe {
            asm!(
                "rep movsb",
                inout("rdi") $dst => _,
                inout("rsi") $src => _,
                inout("rcx") $count => _,
                options(nostack, preserves_flags)
            );
        }
    }};
}

macro_rules! do_memset_mmio_impl {
    (dst: $dst:ident, value: $value:ident, count: $count:ident) => {{
        if $count == 0 { return; }

        unsafe {
            asm!(
                "rep stosb",
                inout("rdi") $dst => _,
                inout("rcx") $count => _,
                in("al") $value,
                options(nostack, preserves_flags)
            );
        }
    }};
}

pub(crate) unsafe fn copy_from_mmio(dst_ptr: *mut u8, src_io_ptr: *const u8, count: usize) {
    if_tdx_enabled!({
        return unsafe { cvm::copy_from_mmio(dst_ptr, src_io_ptr, count) };
    });

    do_copy_mmio_impl!(dst: dst_ptr, src: src_io_ptr, count: count);
}

pub(crate) unsafe fn copy_to_mmio(src_ptr: *const u8, dst_io_ptr: *mut u8, count: usize) {
    if_tdx_enabled!({
        return unsafe { cvm::copy_to_mmio(src_ptr, dst_io_ptr, count) };
    });

    do_copy_mmio_impl!(dst: dst_io_ptr, src: src_ptr, count: count);
}

pub(crate) unsafe fn copy_from_mmio_fallible(
    dst_ptr: *mut u8,
    src_io_ptr: *const u8,
    count: usize,
) -> usize {
    if_tdx_enabled!({
        return unsafe { cvm::copy_from_mmio_fallible(dst_ptr, src_io_ptr, count) };
    });

    let failed_bytes = unsafe { __memcpy_fallible(dst_ptr, src_io_ptr, count) };
    count - failed_bytes
}

pub(crate) unsafe fn copy_to_mmio_fallible(
    src_ptr: *const u8,
    dst_io_ptr: *mut u8,
    count: usize,
) -> usize {
    if_tdx_enabled!({
        return unsafe { cvm::copy_to_mmio_fallible(src_ptr, dst_io_ptr, count) };
    });

    let failed_bytes = unsafe { __memcpy_fallible(dst_io_ptr, src_ptr, count) };
    count - failed_bytes
}

pub(crate) unsafe fn memset_mmio(dst_io_ptr: *mut u8, value: u8, count: usize) {
    if_tdx_enabled!({
        return unsafe { cvm::memset_mmio(dst_io_ptr, value, count) };
    });

    do_memset_mmio_impl!(dst: dst_io_ptr, value: value, count: count);
}
