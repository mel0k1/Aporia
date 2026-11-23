#![expect(dead_code)]

pub mod boot;
pub mod cpu;
pub mod device;
pub(crate) mod io;
pub(crate) mod iommu;
pub mod irq;
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

    let mut io_mem_builder = unsafe { io::construct_io_mem_allocator_builder() };

    irq::chip::init(&mut io_mem_builder);

    timer::init();

    unsafe { crate::boot::smp::boot_all_aps() };

    unsafe { crate::io::init(io_mem_builder) };

    power::init();
}

pub(crate) unsafe fn init_on_ap() {
    unimplemented!()
}

pub fn tsc_freq() -> u64 {
    use core::arch::asm;

    let cntfrq;
    unsafe { asm!("mrs {}, cntfrq_el0", out(reg) cntfrq) };
    cntfrq
}

pub fn read_tsc() -> u64 {
    use core::arch::asm;

    let cntvct;
    unsafe { asm!("mrs {}, cntvct_el0", out(reg) cntvct) };
    cntvct
}

pub fn read_random() -> Option<u64> {
    None
}

pub(crate) fn enable_cpu_features() {
    use core::arch::asm;

    unsafe {
        asm!(
            "mov {tmp}, #(3 << 20)",
            "msr cpacr_el1, {tmp}",
            "isb",
            tmp = out(reg) _,
        );
    }

    unsafe {
        asm!(
            "mov {tmp}, #(1 << 1)",
            "msr cntkctl_el1, {tmp}",
            "isb",
            tmp = out(reg) _,
        );
    }
}
