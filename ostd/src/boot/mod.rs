#![cfg_attr(
    any(
        target_arch = "riscv64",
        target_arch = "loongarch64",
        target_arch = "aarch64"
    ),
    expect(dead_code)
)]

pub mod memory_region;
pub mod smp;

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use memory_region::{MemoryRegion, MemoryRegionArray};
use spin::Once;

use crate::log::LevelFilter;

pub struct BootInfo {
    pub bootloader_name: String,
    pub kernel_cmdline: String,
    pub initramfs: Option<&'static [u8]>,
    pub framebuffer_arg: Option<BootloaderFramebufferArg>,
    pub memory_regions: Vec<MemoryRegion>,
}

pub fn boot_info() -> &'static BootInfo {
    INFO.get().unwrap()
}

static INFO: Once<BootInfo> = Once::new();

#[derive(Clone, Copy, Debug)]
pub enum BootloaderAcpiArg {
    NotProvided,
    ScanBios,
    Rsdp(usize),
    Rsdt(usize),
    Xsdt(usize),
}

#[derive(Clone, Copy, Debug)]
pub struct BootloaderFramebufferArg {
    pub address: usize,
    pub width: usize,
    pub height: usize,
    pub bpp: usize,
}

pub(crate) struct EarlyBootInfo {
    pub(crate) bootloader_name: &'static str,
    pub(crate) kernel_cmdline: &'static str,
    pub(crate) initramfs: Option<&'static [u8]>,
    pub(crate) acpi_arg: BootloaderAcpiArg,
    pub(crate) framebuffer_arg: Option<BootloaderFramebufferArg>,
    pub(crate) memory_regions: MemoryRegionArray,
}

pub(crate) static EARLY_INFO: Once<EarlyBootInfo> = Once::new();

pub(crate) fn init_after_heap() {
    let boot_time_info = EARLY_INFO.get().unwrap();

    INFO.call_once(|| BootInfo {
        bootloader_name: boot_time_info.bootloader_name.to_string(),
        kernel_cmdline: boot_time_info.kernel_cmdline.to_string(),
        initramfs: boot_time_info.initramfs,
        framebuffer_arg: boot_time_info.framebuffer_arg,
        memory_regions: boot_time_info.memory_regions.to_vec(),
    });
}

pub struct EarlyCmdline {
    pub log_level: LevelFilter,
    pub has_early_console: bool,
}

#[linkage = "weak"]
#[unsafe(no_mangle)]
fn __early_cmdline_parser(_cmdline: &str) -> EarlyCmdline {
    EarlyCmdline {
        log_level: LevelFilter::Debug,
        has_early_console: true,
    }
}

pub(crate) fn parse_early_cmdline() -> EarlyCmdline {
    let kernel_cmdline = EARLY_INFO.get().unwrap().kernel_cmdline;
    __early_cmdline_parser(kernel_cmdline)
}

pub(crate) unsafe fn start_kernel() -> ! {
    unsafe extern "Rust" {
        fn __ostd_main() -> !;
    }

    unsafe { crate::init() };

    unsafe { __ostd_main() };
}
