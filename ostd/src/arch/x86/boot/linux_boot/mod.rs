use linux_boot_params::{BootParams, E820Type, EfiInfo, LINUX_BOOT_HEADER_MAGIC};

use super::ToEarlyBootInfo;
use crate::{
    arch::if_tdx_enabled,
    boot::{
        BootloaderAcpiArg, BootloaderFramebufferArg,
        memory_region::{MemoryRegion, MemoryRegionArray, MemoryRegionType},
    },
    mm::kspace::paddr_to_vaddr,
};

fn is_efi_boot(boot_params: &BootParams) -> bool {
    const EFI32_LOADER_SIGNATURE: u32 = u32::from_le_bytes(*b"EL32");
    const EFI64_LOADER_SIGNATURE: u32 = u32::from_le_bytes(*b"EL64");

    let efi_info = boot_params.efi_info;
    matches!(
        efi_info.efi_loader_signature,
        EFI32_LOADER_SIGNATURE | EFI64_LOADER_SIGNATURE | EfiInfo::TDX_LOADER_SIGNATURE
    )
}

impl From<E820Type> for MemoryRegionType {
    fn from(value: E820Type) -> Self {
        match value {
            E820Type::RAM => Self::Usable,
            E820Type::RESERVED => Self::Reserved,
            E820Type::ACPI => Self::Reclaimable,
            E820Type::NVS => Self::NonVolatileSleep,
            E820Type::UNUSABLE => Self::BadMemory,
            _ => Self::Reserved,
        }
    }
}

impl ToEarlyBootInfo for BootParams {
    fn bootloader_name(&self) -> &'static str {
        match self.hdr.type_of_loader {
            0x0 => "LILO",
            0x1 => "Loadlin",
            0x2 => "bootsect-loader",
            0x3 => "Syslinux",
            0x4 => "Etherboot/gPXE/iPXE",
            0x5 => "ELILO",
            0x7 => "GRUB",
            0x8 => "U-Boot",
            0x9 => "Xen",
            0xA => "Gujin",
            0xB => "Qemu",
            0xC => "Arcturus Networks uCbootloader",
            0xD => "kexec-tools",
            0xE => "Extended loader",
            0xF => "Special",
            0x10 => "Reserved",
            0x11 => "Minimal Linux Bootloader <http://sebastian-plotz.blogspot.de>",
            0x12 => "OVMF UEFI virtualization stack",
            _ => "Unknown Linux Loader",
        }
    }

    fn kernel_commandline(&self) -> Option<&'static str> {
        if self.ext_cmd_line_ptr != 0 {
            return None;
        }

        if self.hdr.cmd_line_ptr == 0 || self.hdr.cmdline_size == 0 {
            return None;
        }

        let cmdline_ptr = paddr_to_vaddr(self.hdr.cmd_line_ptr as usize) as *const u8;
        let cmdline_max_len = self.hdr.cmdline_size as usize;

        let mut len = 0;
        while len < cmdline_max_len {
            if unsafe { cmdline_ptr.add(len).read() } == 0 {
                break;
            }
            len += 1;
        }

        let cmdline = unsafe { core::slice::from_raw_parts(cmdline_ptr, len) };

        core::str::from_utf8(cmdline).ok()
    }

    fn initramfs(&self) -> Option<&'static [u8]> {
        if self.ext_ramdisk_image != 0 || self.ext_ramdisk_size != 0 {
            return None;
        }

        if self.hdr.ramdisk_image == 0 || self.hdr.ramdisk_size == 0 {
            return None;
        }

        let initramfs_ptr = paddr_to_vaddr(self.hdr.ramdisk_image as usize);
        let initramfs_len = self.hdr.ramdisk_size as usize;
        let initramfs =
            unsafe { core::slice::from_raw_parts(initramfs_ptr as *const u8, initramfs_len) };

        Some(initramfs)
    }

    fn acpi_arg(&self) -> BootloaderAcpiArg {
        let rsdp = self.acpi_rsdp_addr;

        if rsdp == 0 {
            if is_efi_boot(self) {
                BootloaderAcpiArg::NotProvided
            } else {
                BootloaderAcpiArg::ScanBios
            }
        } else {
            BootloaderAcpiArg::Rsdp(rsdp.try_into().expect("RSDP address overflowed!"))
        }
    }

    fn framebuffer_arg(&self) -> Option<BootloaderFramebufferArg> {
        let screen_info = self.screen_info;

        let address = screen_info.lfb_base as usize | ((screen_info.ext_lfb_base as usize) << 32);
        if address == 0 {
            return None;
        }

        Some(BootloaderFramebufferArg {
            address,
            width: screen_info.lfb_width as usize,
            height: screen_info.lfb_height as usize,
            bpp: screen_info.lfb_depth as usize,
        })
    }

    fn memory_regions(
        &self,
        initramfs: Option<&'static [u8]>,
        kernel_cmdline: Option<&'static str>,
        framebuffer_arg: Option<BootloaderFramebufferArg>,
    ) -> MemoryRegionArray {
        let mut regions = MemoryRegionArray::new();

        let num_entries = self.e820_entries as usize;
        for e820_entry in &self.e820_table[0..num_entries] {
            regions
                .push(MemoryRegion::new(
                    e820_entry.addr.try_into().unwrap(),
                    e820_entry.size.try_into().unwrap(),
                    e820_entry.typ.into(),
                ))
                .unwrap();
        }

        if_tdx_enabled!({
            if let Some(range) = crate::mm::frame::unaccepted::table_memory_range() {
                regions
                    .push(MemoryRegion::new(
                        range.start,
                        range.len(),
                        MemoryRegionType::Reserved,
                    ))
                    .unwrap();
            }
        });

        if_tdx_enabled!({
            regions
                .push(MemoryRegion::new(
                    0x800000,
                    0x006000,
                    MemoryRegionType::NonVolatileSleep,
                ))
                .unwrap();
        });

        super::finish_memory_regions(regions, framebuffer_arg, initramfs, kernel_cmdline)
    }
}

#[unsafe(no_mangle)]
unsafe extern "sysv64" fn __linux_boot(params_ptr: *const BootParams) -> ! {
    let params = unsafe { &*params_ptr };
    assert_eq!({ params.hdr.header }, LINUX_BOOT_HEADER_MAGIC);

    use crate::boot::{EARLY_INFO, start_kernel};

    #[cfg(feature = "cvm_guest")]
    crate::arch::init_cvm_guest(params);

    EARLY_INFO.call_once(|| params.to_early_boot_info());

    unsafe { start_kernel() };
}
