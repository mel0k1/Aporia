use alloc::{boxed::Box, collections::btree_map::BTreeMap, vec::Vec};

use spin::Once;

use crate::{
    arch::irq::HwCpuId,
    mm::{
        FrameAllocOptions, HasPaddrRange, PAGE_SIZE,
        frame::{Segment, meta::KernelMeta},
        paddr_to_vaddr,
    },
    sync::SpinLock,
    task::Task,
    util::id_set::Id,
};

static AP_BOOT_INFO: Once<ApBootInfo> = Once::new();

const AP_BOOT_STACK_SIZE: usize = PAGE_SIZE * 64;

struct ApBootInfo {
    per_ap_raw_info: Box<[PerApRawInfo]>,
    #[cfg_attr(target_arch = "x86_64", expect(dead_code))]
    per_ap_info: Box<[PerApInfo]>,
}

struct PerApInfo {
    #[cfg_attr(target_arch = "x86_64", expect(dead_code))]
    boot_stack_pages: Segment<KernelMeta>,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct PerApRawInfo {
    stack_top: *mut u8,
    cpu_local: *mut u8,
}

unsafe impl Send for PerApRawInfo {}
unsafe impl Sync for PerApRawInfo {}

static HW_CPU_ID_MAP: SpinLock<BTreeMap<u32, HwCpuId>> = SpinLock::new(BTreeMap::new());

pub(crate) unsafe fn boot_all_aps() {
    report_online_and_hw_cpu_id(crate::cpu::CpuId::bsp().as_usize().try_into().unwrap());

    let num_cpus = crate::cpu::num_cpus();

    if num_cpus == 1 {
        return;
    }
    crate::info!("Booting {} processors", num_cpus - 1);

    let mut per_ap_raw_info = Vec::with_capacity(num_cpus);
    let mut per_ap_info = Vec::with_capacity(num_cpus);

    for ap in 1..num_cpus {
        let boot_stack_pages = FrameAllocOptions::new()
            .zeroed(false)
            .alloc_segment_with(AP_BOOT_STACK_SIZE / PAGE_SIZE, |_| KernelMeta)
            .unwrap();

        per_ap_raw_info.push(PerApRawInfo {
            stack_top: paddr_to_vaddr(boot_stack_pages.end_paddr()) as *mut u8,
            cpu_local: paddr_to_vaddr(crate::cpu::local::get_ap(ap.try_into().unwrap())) as *mut u8,
        });
        per_ap_info.push(PerApInfo { boot_stack_pages });
    }

    assert!(!AP_BOOT_INFO.is_completed());
    AP_BOOT_INFO.call_once(move || ApBootInfo {
        per_ap_raw_info: per_ap_raw_info.into_boxed_slice(),
        per_ap_info: per_ap_info.into_boxed_slice(),
    });

    crate::info!("Booting all application processors...");

    let info_ptr = AP_BOOT_INFO.get().unwrap().per_ap_raw_info.as_ptr();
    let pt_ptr = crate::mm::page_table::boot_pt::with_borrow(|pt| pt.root_address()).unwrap();
    unsafe { crate::arch::boot::smp::bringup_all_aps(info_ptr, pt_ptr, num_cpus as u32) };

    wait_for_all_aps_started(num_cpus);

    crate::info!("All application processors started. The BSP continues to run.");
}

static AP_LATE_ENTRY: Once<fn()> = Once::new();

pub fn register_ap_entry(entry: fn()) {
    AP_LATE_ENTRY.call_once(|| entry);
}

#[unsafe(no_mangle)]
pub(crate) unsafe extern "C" fn ap_early_entry(cpu_id: u32) -> ! {
    unsafe { crate::cpu::init_on_ap(cpu_id) };

    crate::arch::enable_cpu_features();

    unsafe { crate::arch::trap::init_on_cpu() };

    unsafe { crate::mm::kspace::activate_kernel_page_table() };

    unsafe { crate::arch::init_on_ap() };

    crate::arch::irq::enable_local();

    unsafe { crate::mm::page_table::boot_pt::dismiss() };

    crate::info!("Processor {} started. Spinning for tasks.", cpu_id);

    report_online_and_hw_cpu_id(cpu_id);

    #[cfg(target_arch = "x86_64")]
    crate::if_tdx_enabled!({
        unsafe { crate::mm::frame::unaccepted::accept_memory_on_ap() };
    });

    let ap_late_entry = AP_LATE_ENTRY.wait();
    crate::arch::mm::tlb_flush_all_excluding_global();

    ap_late_entry();

    Task::yield_now();
    unreachable!("`yield_now` in the boot context should not return");
}

fn report_online_and_hw_cpu_id(cpu_id: u32) {
    let hw_cpu_id = HwCpuId::read_current(&crate::task::disable_preempt());

    let old_val = HW_CPU_ID_MAP.lock().insert(cpu_id, hw_cpu_id);
    assert!(old_val.is_none());
}

fn wait_for_all_aps_started(num_cpus: usize) {
    fn is_all_aps_started(num_cpus: usize) -> bool {
        HW_CPU_ID_MAP.lock().len() == num_cpus
    }

    while !is_all_aps_started(num_cpus) {
        core::hint::spin_loop();
    }
}

pub(crate) fn construct_hw_cpu_id_mapping() -> Box<[HwCpuId]> {
    let mut hw_cpu_id_map = HW_CPU_ID_MAP.lock();
    assert_eq!(hw_cpu_id_map.len(), crate::cpu::num_cpus());

    let result = hw_cpu_id_map
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .into_boxed_slice();
    hw_cpu_id_map.clear();

    result
}
