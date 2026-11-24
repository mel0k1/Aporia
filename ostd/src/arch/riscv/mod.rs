//! Platform-specific code for the RISC-V platform.

#![expect(dead_code)]

pub mod boot;
pub mod cpu;
pub mod device;
pub(crate) mod io;
pub(crate) mod iommu;
pub mod irq;
pub(crate) mod mm;
pub mod power;
pub(crate) mod serial;
pub(crate) mod task;
mod timer;
pub mod trap;

#[cfg(feature = "cvm_guest")]
pub(crate) fn init_cvm_guest() {}

/// Architecture-specific initialization on the bootstrapping processor.
///
/// It should be called when the heap and frame allocators are available.
///
/// # Safety
///
/// 1. This function must be called only once in the boot context of the
///    bootstrapping processor.
/// 2. This function must be called after the kernel page table is activated on
///    the bootstrapping processor.
pub(crate) unsafe fn late_init_on_bsp() {
    unsafe { trap::init_on_cpu() };

    let io_mem_builder = unsafe { io::construct_io_mem_allocator_builder() };

    unsafe { irq::chip::init_on_bsp(&io_mem_builder) };

    unsafe { irq::ipi::init_on_bsp() };

    unsafe { timer::init_on_bsp() };

    unsafe { crate::boot::smp::boot_all_aps() };

    unsafe { crate::io::init(io_mem_builder) };
}

/// Initializes application-processor-specific state.
///
/// On RISC-V, Application Processors (APs) are harts that are not the
/// bootstrapping hart.
///
/// # Safety
///
/// 1. This function must be called only once on each application processor.
/// 2. This function must be called after the BSP's call to [`late_init_on_bsp`]
///    and before any other architecture-specific code in this module is called
///    on this AP.
pub(crate) unsafe fn init_on_ap() {
    unsafe { trap::init_on_cpu() };

    unsafe { irq::chip::init_on_ap() };

    unsafe { irq::ipi::init_on_ap() };

    unsafe { timer::init_on_ap() };
}

/// Returns the frequency of TSC. The unit is Hz.
pub fn tsc_freq() -> u64 {
    timer::get_timebase_freq()
}

/// Reads the current value of the processor's time-stamp counter (TSC).
pub fn read_tsc() -> u64 {
    riscv::register::time::read64()
}

/// Reads a hardware generated 64-bit random value.
///
/// Returns `None` if no random value was generated.
///
/// This implementation uses the RISC-V Entropy Source Extension (Zkr) which
/// provides access to a physical entropy source via the `seed` CSR.
///
/// Reference:
/// - "RISC-V Cryptography Extensions Volume I: Scalar & Entropy Source Instructions"
///   - <https://docs.riscv.org/reference/isa/extensions/crypto-scalar/_attachments/riscv-crypto-spec-scalar.pdf>
///   - Section 2.9: Zkr - Entropy Source Extension
///   - Section 4.1: The `seed` CSR
pub fn read_random() -> Option<u64> {
    use cpu::extension::{IsaExtensions, has_extensions};

    if !has_extensions(IsaExtensions::ZKR) {
        return None;
    }

    const RETRY_LIMIT: usize = 10;

    const OPST_SHIFT: usize = 30;
    const OPST_MASK: usize = 0b11 << OPST_SHIFT;
    const ENTROPY_MASK: usize = 0xFFFF;

    const OPST_ES16: usize = 0b10;
    const OPST_WAIT: usize = 0b01;
    const OPST_BIST: usize = 0b00;
    const OPST_DEAD: usize = 0b11;

    let read_entropy = || -> Option<u16> {
        for _ in 0..RETRY_LIMIT {
            let seed_val: usize;

            unsafe {
                core::arch::asm!(
                    "csrrw {0}, seed, zero",
                    out(reg) seed_val,
                    options(nomem, nostack)
                );
            }

            let status = (seed_val & OPST_MASK) >> OPST_SHIFT;

            if status == OPST_ES16 {
                return Some((seed_val & ENTROPY_MASK) as u16);
            } else if status == OPST_DEAD {
                return None;
            } else if status == OPST_WAIT || status == OPST_BIST {
                core::hint::spin_loop();
            }
        }
        None
    };

    let result = (read_entropy()? as u64)
        | ((read_entropy()? as u64) << 16)
        | ((read_entropy()? as u64) << 32)
        | ((read_entropy()? as u64) << 48);

    Some(result)
}

pub(crate) fn enable_cpu_features() {
    cpu::extension::init();
}
