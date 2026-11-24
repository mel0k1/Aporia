//! CPU-related definitions.

mod id;
pub mod local;

pub use id::{AtomicCpuSet, CpuId, CpuIdFromIntError, CpuSet, PinCurrentCpu, all_cpus, num_cpus};

/// The CPU privilege level: user mode or kernel mode.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum PrivilegeLevel {
    /// User mode.
    User = 0,
    /// Kernel mode.
    Kernel = 1,
}

/// Initializes the CPU module (the BSP part).
///
/// # Safety
///
/// The caller must ensure that
/// 1. We're in the boot context of the BSP and APs have not yet booted.
/// 2. The number of CPUs is available.
/// 3. CPU-local storage has NOT been used.
pub(crate) unsafe fn init_on_bsp() {
    let num_cpus = crate::arch::boot::smp::count_processors().unwrap_or(1);

    unsafe { local::copy_bsp_for_ap(num_cpus as usize) };

    unsafe { id::init_on_bsp(num_cpus) };
}

/// Initializes the CPU module (the AP part).
///
/// # Safety
///
/// The caller must ensure that:
/// 1. We're in the boot context of an AP.
/// 2. The CPU ID of the AP is correct.
pub(crate) unsafe fn init_on_ap(cpu_id: u32) {
    unsafe { id::init_on_ap(cpu_id) };
}
