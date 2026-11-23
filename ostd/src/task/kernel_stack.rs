use core::sync::atomic::Ordering;

use crate::{
    arch::mm::tlb_flush_addr_range,
    cpu::{AtomicCpuSet, CpuSet, PinCurrentCpu},
    impl_frame_meta_for,
    irq::DisabledLocalIrqGuard,
    mm::{
        FrameAllocOptions, PAGE_SIZE,
        kspace::kvirt_area::KVirtArea,
        page_prop::{CachePolicy, PageFlags, PageProperty, PrivilegedPageFlags},
    },
    prelude::*,
};

const STACK_SIZE_IN_PAGES: u32 =
    if let Some(size_str) = option_env!("OSTD_TASK_STACK_SIZE_IN_PAGES") {
        match u32::from_str_radix(size_str, 10) {
            Ok(size) => size,
            Err(_) => panic!(
                "The environment variable `OSTD_TASK_STACK_SIZE_IN_PAGES` \
                    specifies an invalid value"
            ),
        }
    } else {
        DEFAULT_STACK_SIZE_IN_PAGES
    };

const DEFAULT_STACK_SIZE_IN_PAGES: u32 = 128;

const KERNEL_STACK_SIZE: usize = STACK_SIZE_IN_PAGES as usize * PAGE_SIZE;

#[derive(Debug)]
pub(super) struct KernelStack {
    kvirt_area: KVirtArea,
    tlb_coherent: AtomicCpuSet,
    end_vaddr: Vaddr,
}

#[derive(Debug, Default)]
struct KernelStackMeta;

impl_frame_meta_for!(KernelStackMeta);

impl KernelStack {
    pub(super) fn new_with_guard_page() -> Result<Self> {
        let pages = FrameAllocOptions::new()
            .zeroed(false)
            .alloc_segment_with(KERNEL_STACK_SIZE / PAGE_SIZE, |_| KernelStackMeta)?;
        let prop = PageProperty {
            flags: PageFlags::RW,
            cache: CachePolicy::Writeback,
            priv_flags: PrivilegedPageFlags::empty(),
        };
        let new_kvirt_area = KVirtArea::map_frames(
            KERNEL_STACK_SIZE + 4 * PAGE_SIZE,
            2 * PAGE_SIZE,
            pages.into_iter(),
            prop,
        );
        let mapped_start = new_kvirt_area.range().start + 2 * PAGE_SIZE;
        let mapped_end = mapped_start + KERNEL_STACK_SIZE;
        Ok(Self {
            kvirt_area: new_kvirt_area,
            tlb_coherent: AtomicCpuSet::new(CpuSet::new_empty()),
            end_vaddr: mapped_end,
        })
    }

    pub(super) fn flush_tlb(&self, irq_guard: &DisabledLocalIrqGuard) {
        let cur_cpu = irq_guard.current_cpu();
        if !self.tlb_coherent.contains(cur_cpu, Ordering::Relaxed) {
            tlb_flush_addr_range(&self.kvirt_area.range());
            self.tlb_coherent.add(cur_cpu, Ordering::Relaxed);
        }
    }

    pub(super) fn end_vaddr(&self) -> Vaddr {
        self.end_vaddr
    }
}
