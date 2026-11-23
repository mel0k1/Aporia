mod linux_boot;
mod multiboot;
mod multiboot2;
mod pvh;

pub(crate) mod smp;

use core::{arch::global_asm, num::NonZeroUsize};

use acpi::rsdp::Rsdp;

use crate::{
    arch::kernel::acpi::AcpiMemoryHandler,
    boot::{
        BootloaderAcpiArg, BootloaderFramebufferArg, EarlyBootInfo,
        memory_region::{MemoryRegion, MemoryRegionArray, MemoryRegionType},
    },
};

global_asm!(
    include_str!("bsp_boot.S"),
    KCODE64 = const super::trap::gdt::KCODE64,
    KDATA = const super::trap::gdt::KDATA,
    KCODE32 = const super::trap::gdt::KCODE32,
);
global_asm!(include_str!("ap_boot.S"));

trait ToEarlyBootInfo {
    fn bootloader_name(&self) -> &'static str;

    fn kernel_commandline(&self) -> Option<&'static str>;

    fn initramfs(&self) -> Option<&'static [u8]>;

    fn acpi_arg(&self) -> BootloaderAcpiArg;

    fn framebuffer_arg(&self) -> Option<BootloaderFramebufferArg>;

    fn memory_regions(
        &self,
        initramfs: Option<&'static [u8]>,
        kernel_cmdline: Option<&'static str>,
        framebuffer_arg: Option<BootloaderFramebufferArg>,
    ) -> MemoryRegionArray;

    fn to_early_boot_info(&self) -> EarlyBootInfo {
        let kernel_cmdline = self.kernel_commandline();
        let initramfs = self.initramfs();
        let framebuffer_arg = self.framebuffer_arg();

        EarlyBootInfo {
            bootloader_name: self.bootloader_name(),
            kernel_cmdline: kernel_cmdline.unwrap_or(""),
            initramfs,
            acpi_arg: self.acpi_arg(),
            framebuffer_arg,
            memory_regions: self.memory_regions(initramfs, kernel_cmdline, framebuffer_arg),
        }
    }
}

fn finish_memory_regions(
    mut regions: MemoryRegionArray,
    framebuffer_arg: Option<BootloaderFramebufferArg>,
    initramfs: Option<&'static [u8]>,
    kernel_cmdline: Option<&'static str>,
) -> MemoryRegionArray {
    if let Some(fb) = framebuffer_arg {
        regions.push(MemoryRegion::framebuffer(&fb)).unwrap();
    }

    regions.push(MemoryRegion::kernel()).unwrap();

    if let Some(initramfs) = initramfs {
        regions.push(MemoryRegion::module(initramfs)).unwrap();
    }

    regions.push(smp::reclaimable_memory_region()).unwrap();

    if let Some(kcmdline) = kernel_cmdline {
        regions
            .push(MemoryRegion::module(kcmdline.as_bytes()))
            .unwrap();
    }

    regions.into_non_overlapping()
}

fn find_acpi_root_table_address() -> Option<NonZeroUsize> {
    let Ok(rsdp) = (unsafe { Rsdp::search_for_on_bios(AcpiMemoryHandler {}) }) else {
        return None;
    };

    if rsdp.revision() == 0 {
        NonZeroUsize::new(rsdp.rsdt_address() as usize)
    } else {
        NonZeroUsize::new(rsdp.xsdt_address() as usize)
    }
}

fn effective_region_type(
    typ: MemoryRegionType,
    base: usize,
    len: usize,
    acpi_root_table_address: Option<NonZeroUsize>,
) -> MemoryRegionType {
    if typ == MemoryRegionType::Reserved
        && acpi_root_table_address.is_some_and(|addr| (base..(base + len)).contains(&addr.get()))
    {
        MemoryRegionType::Reclaimable
    } else {
        typ
    }
}
