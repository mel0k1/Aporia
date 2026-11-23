use crate::{
    arch::io::io_mem::{read_once, write_once},
    mm::PodOnce,
};

macro_rules! for_aligned_chunks {
    ($io_addr:expr, $count:expr, $max_chunk:expr, $op:ident ( $($arg:expr),* $(,)? )) => {{
        crate::io::io_mem::util::for_each_chunk_with_max($io_addr, $count, $max_chunk, |chunk_size| {
            match chunk_size {
                1 => {
                    unsafe { $op::<u8>($($arg),*) }
                }
                2 => {
                    unsafe { $op::<u16>($($arg),*) }
                }
                4 => {
                    unsafe { $op::<u32>($($arg),*) }
                }
                8 => {
                    unsafe { $op::<u64>($($arg),*) }
                }
                _ => unreachable!("unexpected chunk size: {}", chunk_size),
            }
        })
    }};
}

pub(crate) use for_aligned_chunks;

#[doc(hidden)]
pub(crate) fn for_each_chunk_with_max(
    mut io_addr: usize,
    count: usize,
    max_chunk: usize,
    mut copy_chunk_fn: impl FnMut(usize) -> usize,
) -> usize {
    debug_assert!(max_chunk.is_power_of_two());
    debug_assert!((1..=8).contains(&max_chunk));

    let mut copied_total = 0;
    let mut remaining = count;
    let max_log2 = max_chunk.trailing_zeros();

    while remaining > 0 {
        let addr_align_log2 = io_addr.trailing_zeros();
        let mut chunk_size = 1 << addr_align_log2.min(max_log2);

        if chunk_size > remaining {
            chunk_size = 1 << (usize::BITS - 1 - remaining.leading_zeros());
        }

        let copied = copy_chunk_fn(chunk_size);
        debug_assert!(copied <= chunk_size);
        copied_total += copied;
        if copied < chunk_size {
            return copied_total;
        }

        remaining -= chunk_size;
        io_addr += chunk_size;
    }

    copied_total
}

pub(crate) unsafe fn copy_from_mmio_val<T: PodOnce>(
    dst_ptr: &mut *mut u8,
    src_io_ptr: &mut *const u8,
) -> usize {
    debug_assert!((*src_io_ptr).addr().is_multiple_of(align_of::<T>()));
    unsafe {
        let val: T = read_once((*src_io_ptr).cast::<T>());
        let dst = *dst_ptr;
        let align_mask = size_of::<T>() - 1;
        if dst.addr() & align_mask == 0 {
            core::ptr::write(dst.cast::<T>(), val);
        } else {
            core::ptr::write_unaligned(dst.cast::<T>(), val);
        }
    }

    *src_io_ptr = (*src_io_ptr).wrapping_add(size_of::<T>());
    *dst_ptr = (*dst_ptr).wrapping_add(size_of::<T>());
    size_of::<T>()
}

pub(crate) unsafe fn copy_to_mmio_val<T: PodOnce>(
    src_ptr: &mut *const u8,
    dst_io_ptr: &mut *mut u8,
) -> usize {
    debug_assert!((*dst_io_ptr).addr().is_multiple_of(align_of::<T>()));
    unsafe {
        let src = *src_ptr;
        let align_mask = size_of::<T>() - 1;
        let val: T = if src.addr() & align_mask != 0 {
            core::ptr::read_unaligned(src.cast::<T>())
        } else {
            core::ptr::read(src.cast::<T>())
        };
        write_once((*dst_io_ptr).cast::<T>(), val);
    }

    *src_ptr = (*src_ptr).wrapping_add(size_of::<T>());
    *dst_io_ptr = (*dst_io_ptr).wrapping_add(size_of::<T>());
    size_of::<T>()
}

pub(crate) unsafe fn memset_mmio_val<T: PodOnce>(dst_io_ptr: &mut *mut u8, value: u8) -> usize {
    debug_assert!((*dst_io_ptr).addr().is_multiple_of(align_of::<T>()));

    let repeated = u64::from_ne_bytes([value; 8]);
    unsafe {
        match size_of::<T>() {
            1 => write_once((*dst_io_ptr).cast::<u8>(), repeated as u8),
            2 => write_once((*dst_io_ptr).cast::<u16>(), repeated as u16),
            4 => write_once((*dst_io_ptr).cast::<u32>(), repeated as u32),
            8 => write_once((*dst_io_ptr).cast::<u64>(), repeated),
            _ => core::hint::unreachable_unchecked(),
        }
    }

    *dst_io_ptr = (*dst_io_ptr).wrapping_add(size_of::<T>());
    size_of::<T>()
}
