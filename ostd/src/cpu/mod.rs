mod id;
pub mod local;

pub use id::{AtomicCpuSet, CpuId, CpuIdFromIntError, CpuSet, PinCurrentCpu, all_cpus, num_cpus};

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum PrivilegeLevel {
    User = 0,
    Kernel = 1,
}

pub(crate) unsafe fn init_on_bsp() {
    let num_cpus = crate::arch::boot::smp::count_processors().unwrap_or(1);

    unsafe { local::copy_bsp_for_ap(num_cpus as usize) };

    unsafe { id::init_on_bsp(num_cpus) };
}

pub(crate) unsafe fn init_on_ap(cpu_id: u32) {
    unsafe { id::init_on_ap(cpu_id) };
}
