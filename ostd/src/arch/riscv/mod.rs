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

pub(crate) unsafe fn late_init_on_bsp() {
    unsafe { trap::init_on_cpu() };

    let io_mem_builder = unsafe { io::construct_io_mem_allocator_builder() };

    unsafe { irq::chip::init_on_bsp(&io_mem_builder) };

    unsafe { irq::ipi::init_on_bsp() };

    unsafe { timer::init_on_bsp() };

    unsafe { crate::boot::smp::boot_all_aps() };

    unsafe { crate::io::init(io_mem_builder) };
}

pub(crate) unsafe fn init_on_ap() {
    unsafe { trap::init_on_cpu() };

    unsafe { irq::chip::init_on_ap() };

    unsafe { irq::ipi::init_on_ap() };

    unsafe { timer::init_on_ap() };
}

pub fn tsc_freq() -> u64 {
    timer::get_timebase_freq()
}

pub fn read_tsc() -> u64 {
    riscv::register::time::read64()
}

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
