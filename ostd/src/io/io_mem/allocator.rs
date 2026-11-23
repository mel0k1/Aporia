use alloc::vec::Vec;
use core::ops::Range;

use spin::Once;

use crate::{
    debug, info,
    io::io_mem::{Insensitive, IoMem, Sensitive},
    mm::{CachePolicy, PageFlags},
    util::range_alloc::RangeAllocator,
};

pub(super) struct IoMemAllocator {
    allocators: Vec<RangeAllocator>,
}

impl IoMemAllocator {
    pub(super) fn acquire(
        &self,
        range: Range<usize>,
        cache_policy: CachePolicy,
    ) -> Option<IoMem<Insensitive>> {
        debug!(
            "Try to acquire security-insensitive MMIO range: {:#x?} with cache policy: {:?}",
            range, cache_policy
        );

        find_allocator(&self.allocators, &range)?
            .alloc_specific(&range)
            .ok()?;

        unsafe { Some(IoMem::new(range, PageFlags::RW, cache_policy)) }
    }

    #[expect(dead_code)]
    pub(super) unsafe fn recycle(&self, range: Range<usize>) {
        debug!("Recycling MMIO range: {:#x?}", range);

        let allocator = find_allocator(&self.allocators, &range).unwrap();

        allocator.free(range);
    }

    unsafe fn new(allocators: Vec<RangeAllocator>) -> Self {
        Self { allocators }
    }
}

pub(crate) struct IoMemAllocatorBuilder {
    allocators: Vec<RangeAllocator>,
}

impl IoMemAllocatorBuilder {
    pub(crate) unsafe fn new(ranges: Vec<Range<usize>>) -> Self {
        info!(
            "Creating new I/O memory allocator builder, ranges: {:#x?}",
            ranges
        );
        let mut allocators = Vec::with_capacity(ranges.len());
        for range in ranges {
            allocators.push(RangeAllocator::new(range));
        }
        Self { allocators }
    }

    #[cfg_attr(target_arch = "loongarch64", expect(unused))]
    pub(crate) fn reserve(
        &self,
        range: Range<usize>,
        cache_policy: CachePolicy,
    ) -> IoMem<Sensitive> {
        debug!(
            "Try to reserve security-sensitive MMIO range: {:#x?} with cache policy: {:?}",
            range, cache_policy
        );

        self.remove(range.start..range.end);

        unsafe { IoMem::new(range, PageFlags::RW, cache_policy) }
    }

    pub(crate) fn remove(&self, range: Range<usize>) {
        let Some(allocator) = find_allocator(&self.allocators, &range) else {
            panic!(
                "Allocator for the system device's MMIO was not found. Range: {:x?}",
                range
            );
        };

        if let Err(err) = allocator.alloc_specific(&range) {
            panic!(
                "An error occurred while trying to remove access to the system device's MMIO. Range: {:x?}. Error: {:?}",
                range, err
            );
        }
    }
}

pub(super) static IO_MEM_ALLOCATOR: Once<IoMemAllocator> = Once::new();

pub(in crate::io) unsafe fn init(io_mem_builder: IoMemAllocatorBuilder) {
    IO_MEM_ALLOCATOR.call_once(|| unsafe { IoMemAllocator::new(io_mem_builder.allocators) });
}

fn find_allocator<'a>(
    allocators: &'a [RangeAllocator],
    range: &Range<usize>,
) -> Option<&'a RangeAllocator> {
    for allocator in allocators.iter() {
        let allocator_range = allocator.fullrange();
        if allocator_range.start >= range.end || allocator_range.end <= range.start {
            continue;
        }

        return Some(allocator);
    }
    None
}

#[cfg(ktest)]
mod test {
    use alloc::vec;

    use super::{IoMemAllocator, IoMemAllocatorBuilder};
    use crate::{
        mm::{CachePolicy, PAGE_SIZE},
        prelude::ktest,
    };

    #[expect(clippy::reversed_empty_ranges)]
    #[expect(clippy::single_range_in_vec_init)]
    #[ktest]
    fn illegal_region() {
        let range = vec![0x4000_0000..0x4200_0000];
        let allocator =
            unsafe { IoMemAllocator::new(IoMemAllocatorBuilder::new(range).allocators) };

        assert!(allocator.acquire(0..0, CachePolicy::Uncacheable).is_none());
        assert!(
            allocator
                .acquire(usize::MAX..0, CachePolicy::Uncacheable)
                .is_none()
        );

        assert!(
            allocator
                .acquire(0x4000_0000..0x4000_0000, CachePolicy::Uncacheable)
                .is_none()
        );
        assert!(
            allocator
                .acquire(0x4000_1000..0x4000_1000, CachePolicy::Uncacheable)
                .is_none()
        );
        assert!(
            allocator
                .acquire(0x41ff_0000..0x41ff_0000, CachePolicy::Uncacheable)
                .is_none()
        );
        assert!(
            allocator
                .acquire(0x4200_0000..0x4200_0000, CachePolicy::Uncacheable)
                .is_none()
        );

        assert!(
            allocator
                .acquire(0x4000_1000..0x4000_0000, CachePolicy::Uncacheable)
                .is_none()
        );
        assert!(
            allocator
                .acquire(0x4000_2000..0x4000_1000, CachePolicy::Uncacheable)
                .is_none()
        );
        assert!(
            allocator
                .acquire(0x41ff_f000..0x41ff_e000, CachePolicy::Uncacheable)
                .is_none()
        );
        assert!(
            allocator
                .acquire(0x4200_0000..0x41ff_f000, CachePolicy::Uncacheable)
                .is_none()
        );
    }

    #[ktest]
    fn conflict_region() {
        let max_paddr = 0x100_000_000_000;

        let io_mem_region_a = max_paddr..max_paddr + 0x200_0000;
        let io_mem_region_b =
            (io_mem_region_a.end + PAGE_SIZE)..(io_mem_region_a.end + 10 * PAGE_SIZE);
        let range = vec![io_mem_region_a.clone(), io_mem_region_b.clone()];

        let allocator =
            unsafe { IoMemAllocator::new(IoMemAllocatorBuilder::new(range).allocators) };

        assert!(
            allocator
                .acquire(
                    (io_mem_region_a.start - 1)..io_mem_region_a.start,
                    CachePolicy::Uncacheable
                )
                .is_none()
        );
        assert!(
            allocator
                .acquire(
                    io_mem_region_a.start..(io_mem_region_a.start + 1),
                    CachePolicy::Uncacheable
                )
                .is_some()
        );

        assert!(
            allocator
                .acquire(
                    (io_mem_region_a.end + 1)..(io_mem_region_b.start - 1),
                    CachePolicy::Uncacheable
                )
                .is_none()
        );
        assert!(
            allocator
                .acquire(
                    (io_mem_region_a.end - 1)..(io_mem_region_b.start + 1),
                    CachePolicy::Uncacheable
                )
                .is_none()
        );

        assert!(
            allocator
                .acquire(
                    (io_mem_region_a.end - 1)..io_mem_region_a.end,
                    CachePolicy::Uncacheable
                )
                .is_some()
        );
        assert!(
            allocator
                .acquire(
                    io_mem_region_a.end..(io_mem_region_a.end + 1),
                    CachePolicy::Uncacheable
                )
                .is_none()
        );
    }
}
