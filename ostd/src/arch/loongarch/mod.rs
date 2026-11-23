#![expect(dead_code)]

pub mod boot;
pub mod cpu;
pub mod device;
pub(crate) mod io;
pub(crate) mod iommu;
pub(crate) mod irq;
pub(crate) mod mm;
pub mod power;
pub mod serial;
pub(crate) mod task;
mod timer;
pub mod trap;

#[cfg(feature = "cvm_guest")]
pub(crate) fn init_cvm_guest() {}

pub(crate) unsafe fn late_init_on_bsp() {
    unsafe { trap::init_on_cpu() };

    let io_mem_builder = unsafe { io::construct_io_mem_allocator_builder() };

    irq::chip::init();

    unsafe { crate::boot::smp::boot_all_aps() };

    unsafe { crate::io::init(io_mem_builder) };

    power::init();
}

pub(crate) unsafe fn init_on_ap() {
    unimplemented!()
}

pub fn tsc_freq() -> u64 {
    loongArch64::time::get_timer_freq() as _
}

pub fn read_tsc() -> u64 {
    loongArch64::time::Time::read() as _
}

pub fn read_random() -> Option<u64> {
    None
}

pub(crate) fn enable_cpu_features() {}
