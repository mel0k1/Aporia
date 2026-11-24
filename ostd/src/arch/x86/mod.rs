//! Platform-specific code for the x86 platform.

pub(crate) mod boot;
pub mod cpu;
pub mod device;
pub(crate) mod io;
pub(crate) mod iommu;
pub mod irq;
pub mod kernel;
pub(crate) mod mm;
pub mod power;
pub mod serial;
pub(crate) mod task;
mod timer;
pub mod trap;
pub mod vm;

#[cfg(feature = "cvm_guest")]
pub(crate) mod tdx_guest;

#[cfg(feature = "cvm_guest")]
use crate::mm::frame::unaccepted;

#[cfg(feature = "cvm_guest")]
pub(crate) fn init_cvm_guest(boot_params: &linux_boot_params::BootParams) {
    use ::tdx_guest::{
        SeptVeError, disable_sept_ve, init_tdx, metadata, reduce_unnecessary_ve,
        tdcall::{InitError, write_td_metadata},
        tdvmcall::report_fatal_error_simple,
    };

    match init_tdx() {
        Ok(td_info) => {
            reduce_unnecessary_ve().unwrap();
            match disable_sept_ve(td_info.attributes) {
                Ok(_) => {}
                Err(SeptVeError::Misconfiguration) => {
                    crate::early_println!(
                        "[kernel] Error: TD misconfiguration: \
                        The SEPT_VE_DISABLE bit of the TD attributes must be set by VMM \
                        when running in non-debug mode and FLEXIBLE_PENDING_VE is not enabled."
                    );
                    report_fatal_error_simple("TD misconfiguration: SEPT #VE has to be disabled");
                }
                Err(e) => {
                    crate::early_println!("[kernel] Error: Unexpected TDX error: {:?}", e);
                    report_fatal_error_simple(
                        "Disabling SEPT #VE failed due to unexpected TDX error",
                    );
                }
            }

            write_td_metadata(metadata::NOTIFY_ENABLES, 1, 1).unwrap();

            crate::early_println!(
                "[kernel] Intel TDX initialized\n[kernel] td gpaw: {}, td attributes: {:?}",
                td_info.gpaw,
                td_info.attributes
            );
            unaccepted::init(boot_params);
        }
        Err(InitError::TdxGetVpInfoError(td_call_error)) => {
            crate::early_println!(
                "[kernel] Intel TDX not initialized, Failed to get TD info. TD call error: {:?}",
                td_call_error
            );
            report_fatal_error_simple("Intel TDX not initialized, Failed to get TD info.");
        }

        Err(_) => {}
    }
}

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

    kernel::apic::init(&io_mem_builder).expect("APIC doesn't exist");
    irq::chip::init(&io_mem_builder);
    irq::ipi::init();

    kernel::tsc::init_tsc_freq();
    timer::init_on_bsp();

    unsafe { crate::boot::smp::boot_all_aps() };

    if_tdx_enabled!({
        unsafe { unaccepted::accept_memory_on_bsp() };
    });

    if_tdx_enabled!({
    } else {
        match iommu::init(&io_mem_builder) {
            Ok(_) => {}
            Err(err) => crate::warn!("IOMMU initialization error: {:?}", err),
        }
    });

    unsafe { crate::io::init(io_mem_builder) };

    kernel::acpi::init();
    power::init();
}

/// Initializes application-processor-specific state.
///
/// # Safety
///
/// 1. This function must be called only once on each application processor.
/// 2. This function must be called after the BSP's call to [`late_init_on_bsp`]
///    and before any other architecture-specific code in this module is called
///    on this AP.
pub(crate) unsafe fn init_on_ap() {
    timer::init_on_ap();
}

/// Returns the frequency of TSC. The unit is Hz.
pub fn tsc_freq() -> u64 {
    use core::sync::atomic::Ordering;

    kernel::tsc::TSC_FREQ.load(Ordering::Acquire)
}

/// Reads the current value of the processor's time-stamp counter (TSC).
pub fn read_tsc() -> u64 {
    use core::arch::x86_64::_rdtsc;

    unsafe { _rdtsc() }
}

/// Reads a hardware generated 64-bit random value.
///
/// Returns `None` if no random value was generated.
pub fn read_random() -> Option<u64> {
    use core::arch::x86_64::_rdrand64_step;

    use cpu::extension::{IsaExtensions, has_extensions};

    if !has_extensions(IsaExtensions::RDRAND) {
        return None;
    }

    const RETRY_LIMIT: usize = 10;

    for _ in 0..RETRY_LIMIT {
        let mut val = 0;
        let generated = unsafe { _rdrand64_step(&mut val) };
        if generated == 1 {
            return Some(val);
        }
    }
    None
}

pub(crate) fn enable_cpu_features() {
    use cpu::extension::{IsaExtensions, has_extensions};
    use x86_64::registers::{
        control::{Cr0Flags, Cr4Flags},
        xcontrol::XCr0Flags,
    };

    cpu::extension::init();
    vm::init();

    let mut cr0 = x86_64::registers::control::Cr0::read();
    cr0 |= Cr0Flags::WRITE_PROTECT;

    cr0 |= Cr0Flags::NUMERIC_ERROR | Cr0Flags::MONITOR_COPROCESSOR;
    unsafe { x86_64::registers::control::Cr0::write(cr0) };

    let mut cr4 = x86_64::registers::control::Cr4::read();
    cr4 |= Cr4Flags::OSFXSR | Cr4Flags::OSXMMEXCPT_ENABLE | Cr4Flags::PAGE_GLOBAL;
    if has_extensions(IsaExtensions::XSAVE) {
        cr4 |= Cr4Flags::OSXSAVE;
    }

    if has_extensions(IsaExtensions::FSGSBASE) {
        cr4 |= Cr4Flags::FSGSBASE;
    }
    unsafe { x86_64::registers::control::Cr4::write(cr4) };

    if has_extensions(IsaExtensions::XSAVE) {
        let mut xcr0 = x86_64::registers::xcontrol::XCr0::read();
        xcr0 |= XCr0Flags::SSE;
        if has_extensions(IsaExtensions::AVX) {
            xcr0 |= XCr0Flags::AVX;
        }
        if has_extensions(IsaExtensions::AVX512F) {
            xcr0 |= XCr0Flags::OPMASK | XCr0Flags::ZMM_HI256 | XCr0Flags::HI16_ZMM;
        }
        unsafe { x86_64::registers::xcontrol::XCr0::write(xcr0) };
    }

    cpu::context::enable_essential_features();

    mm::enable_essential_features();
}

/// Inserts a TDX-specific code block.
///
/// This macro conditionally executes a TDX-specific code block based on the following conditions:
/// (1) The `cvm_guest` feature is enabled at compile time.
/// (2) The TDX feature is detected at runtime via `::tdx_guest::tdx_is_enabled()`.
///
/// If both conditions are met, the `if_block` is executed. If an `else_block` is provided, it will be executed
/// when either the `cvm_guest` feature is not enabled or the TDX feature is not detected at runtime.
#[macro_export]
macro_rules! if_tdx_enabled {
    ($if_block:block else $else_block:block) => {{
        #[cfg(feature = "cvm_guest")]
        {
            if ::tdx_guest::tdx_is_enabled() {
                $if_block
            } else {
                $else_block
            }
        }
        #[cfg(not(feature = "cvm_guest"))]
        {
            $else_block
        }
    }};

    ($if_block:block) => {{
        #[cfg(feature = "cvm_guest")]
        {
            if ::tdx_guest::tdx_is_enabled() {
                $if_block
            }
        }
    }};
}

pub use if_tdx_enabled;
