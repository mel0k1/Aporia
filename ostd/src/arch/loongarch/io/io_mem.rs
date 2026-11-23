use crate::{
    arch::mm::__memcpy_fallible,
    io::io_mem::util::{copy_from_mmio_val, copy_to_mmio_val, for_aligned_chunks, memset_mmio_val},
    mm::PodOnce,
};

pub(crate) unsafe fn read_once<T: PodOnce>(ptr: *const T) -> T {
    unsafe { core::ptr::read_volatile(ptr) }
}

pub(crate) unsafe fn write_once<T: PodOnce>(ptr: *mut T, val: T) {
    unsafe { core::ptr::write_volatile(ptr, val) }
}

pub(crate) unsafe fn copy_from_mmio(mut dst_ptr: *mut u8, mut src_io_ptr: *const u8, count: usize) {
    let copied = for_aligned_chunks!(
        src_io_ptr.addr(),
        count,
        8,
        copy_from_mmio_val(&mut dst_ptr, &mut src_io_ptr)
    );
    debug_assert_eq!(copied, count);
}

pub(crate) unsafe fn copy_to_mmio(mut src_ptr: *const u8, mut dst_io_ptr: *mut u8, count: usize) {
    let copied = for_aligned_chunks!(
        dst_io_ptr.addr(),
        count,
        8,
        copy_to_mmio_val(&mut src_ptr, &mut dst_io_ptr)
    );
    debug_assert_eq!(copied, count);
}

pub(crate) unsafe fn copy_from_mmio_fallible(
    dst_ptr: *mut u8,
    src_io_ptr: *const u8,
    count: usize,
) -> usize {
    let failed_bytes = unsafe { __memcpy_fallible(dst_ptr, src_io_ptr, count) };
    count - failed_bytes
}

pub(crate) unsafe fn copy_to_mmio_fallible(
    src_ptr: *const u8,
    dst_io_ptr: *mut u8,
    count: usize,
) -> usize {
    let failed_bytes = unsafe { __memcpy_fallible(dst_io_ptr, src_ptr, count) };
    count - failed_bytes
}

pub(crate) unsafe fn memset_mmio(mut dst_io_ptr: *mut u8, value: u8, count: usize) {
    let written = for_aligned_chunks!(
        dst_io_ptr.addr(),
        count,
        8,
        memset_mmio_val(&mut dst_io_ptr, value)
    );
    debug_assert_eq!(written, count);
}
