use core::ffi::CStr;

use int_to_c_enum::TryFromInt;

use super::ToEarlyBootInfo;
use crate::{
    boot::{
        BootloaderAcpiArg, BootloaderFramebufferArg, EARLY_INFO,
        memory_region::{MemoryRegion, MemoryRegionArray, MemoryRegionType},
        start_kernel,
    },
    mm::{Paddr, kspace::paddr_to_vaddr},
};

#[cfg(feature = "pvh_boot")]
core::arch::global_asm!(include_str!("note.S"));

const HVM_START_MAGIC_VALUE: u32 = 0x336e_c578;

#[repr(C, packed)]
struct HvmStartInfoV0 {
    magic: u32,
    version: u32,
    flags: u32,
    nr_modules: u32,
    modlist_paddr: u64,
    cmdline_paddr: u64,
    rsdp_paddr: u64,
}

#[repr(C, packed)]
struct HvmStartInfoV1 {
    v0: HvmStartInfoV0,
    memmap_paddr: u64,
    memmap_entries: u32,
    _reserved: u32,
}

#[repr(C, packed)]
struct HvmModlistEntry {
    paddr: u64,
    size: u64,
    cmdline_paddr: u64,
    _reserved: u64,
}

#[repr(C, packed)]
struct HvmMemmapTableEntry {
    addr: u64,
    size: u64,
    typ: u32,
    _reserved: u32,
}

#[repr(u32)]
#[derive(Debug, TryFromInt)]
enum HvmMemmapType {
    Ram = 1,
    Reserved = 2,
    Acpi = 3,
    Nvs = 4,
    Unusable = 5,
    Disabled = 6,
    Pmem = 7,
}

impl From<HvmMemmapType> for MemoryRegionType {
    fn from(typ: HvmMemmapType) -> Self {
        match typ {
            HvmMemmapType::Ram => Self::Usable,
            HvmMemmapType::Reserved | HvmMemmapType::Disabled | HvmMemmapType::Pmem => {
                Self::Reserved
            }
            HvmMemmapType::Acpi => Self::Reclaimable,
            HvmMemmapType::Nvs => Self::NonVolatileSleep,
            HvmMemmapType::Unusable => Self::BadMemory,
        }
    }
}

fn parse_memory_region_type(typ: u32) -> MemoryRegionType {
    HvmMemmapType::try_from(typ)
        .map(MemoryRegionType::from)
        .unwrap_or(MemoryRegionType::Reserved)
}

impl HvmStartInfoV0 {
    fn modules(&self) -> &[HvmModlistEntry] {
        if self.nr_modules == 0 || self.modlist_paddr == 0 {
            return &[];
        }

        unsafe {
            core::slice::from_raw_parts(
                paddr_to_vaddr(self.modlist_paddr as Paddr) as *const HvmModlistEntry,
                self.nr_modules as usize,
            )
        }
    }
}

impl HvmStartInfoV1 {
    fn memory_map(&self) -> Option<&[HvmMemmapTableEntry]> {
        if self.memmap_entries == 0 {
            return None;
        }

        Some(unsafe {
            core::slice::from_raw_parts(
                paddr_to_vaddr(self.memmap_paddr as Paddr) as *const HvmMemmapTableEntry,
                self.memmap_entries as usize,
            )
        })
    }
}

impl ToEarlyBootInfo for HvmStartInfoV0 {
    fn bootloader_name(&self) -> &'static str {
        "PVH-capable VMM"
    }

    fn kernel_commandline(&self) -> Option<&'static str> {
        if self.cmdline_paddr == 0 {
            return None;
        }

        let cmdline =
            unsafe { CStr::from_ptr(paddr_to_vaddr(self.cmdline_paddr as Paddr) as *const _) };

        cmdline.to_str().ok()
    }

    fn initramfs(&self) -> Option<&'static [u8]> {
        let module = self.modules().first()?;

        if module.paddr == 0 || module.size == 0 {
            return None;
        }

        Some(unsafe {
            core::slice::from_raw_parts(
                paddr_to_vaddr(module.paddr as Paddr) as *const u8,
                module.size as usize,
            )
        })
    }

    fn acpi_arg(&self) -> BootloaderAcpiArg {
        if self.rsdp_paddr == 0 {
            BootloaderAcpiArg::ScanBios
        } else {
            BootloaderAcpiArg::Rsdp(
                self.rsdp_paddr
                    .try_into()
                    .expect("RSDP address overflowed!"),
            )
        }
    }

    fn framebuffer_arg(&self) -> Option<BootloaderFramebufferArg> {
        None
    }

    fn memory_regions(
        &self,
        initramfs: Option<&'static [u8]>,
        kernel_cmdline: Option<&'static str>,
        framebuffer_arg: Option<BootloaderFramebufferArg>,
    ) -> MemoryRegionArray {
        let mut regions = MemoryRegionArray::new();

        let acpi_root_table_address = if self.rsdp_paddr == 0 {
            super::find_acpi_root_table_address()
        } else {
            None
        };

        if self.version >= 1 {
            let v1 = unsafe { &*(self as *const HvmStartInfoV0 as *const HvmStartInfoV1) };
            if let Some(memmap) = v1.memory_map() {
                for entry in memmap {
                    let base = entry.addr.try_into().unwrap();
                    let len = entry.size.try_into().unwrap();
                    let typ = super::effective_region_type(
                        parse_memory_region_type(entry.typ),
                        base,
                        len,
                        acpi_root_table_address,
                    );

                    regions.push(MemoryRegion::new(base, len, typ)).unwrap();
                }
            }
        }

        super::finish_memory_regions(regions, framebuffer_arg, initramfs, kernel_cmdline)
    }
}

#[unsafe(no_mangle)]
unsafe extern "sysv64" fn __pvh_entry(start_info_ptr: *const HvmStartInfoV0) -> ! {
    let start_info = unsafe { &*start_info_ptr };

    let magic = start_info.magic;
    assert_eq!(magic, HVM_START_MAGIC_VALUE);

    EARLY_INFO.call_once(|| start_info.to_early_boot_info());

    unsafe { start_kernel() };
}
