use super::ToEarlyBootInfo;
use crate::{
    boot::{
        BootloaderAcpiArg, BootloaderFramebufferArg,
        memory_region::{MemoryRegion, MemoryRegionArray, MemoryRegionType},
    },
    mm::{Paddr, kspace::paddr_to_vaddr},
};

#[cfg(not(feature = "pvh_boot"))]
core::arch::global_asm!(include_str!("header.S"));

const MULTIBOOT_ENTRY_MAGIC: u32 = 0x2BADB002;

#[repr(C, packed)]
#[derive(Clone, Copy, Debug)]
struct MultibootLegacyInfo {
    flags: u32,

    mem_lower: u32,
    mem_upper: u32,

    boot_device: u32,

    cmdline: u32,

    mods_count: u32,
    mods_addr: u32,

    symbols: [u8; 16],

    memory_map_len: u32,
    memory_map_addr: u32,

    drives_length: u32,
    drives_addr: u32,

    config_table: u32,

    boot_loader_name: u32,

    apm_table: u32,

    vbe_table: VbeTable,

    framebuffer_table: FramebufferTable,
}

impl MultibootLegacyInfo {
    fn get_memory_map(&self) -> MemoryEntryIter {
        let cur_ptr = paddr_to_vaddr(self.memory_map_addr as Paddr) as *const u8;
        let region_end = unsafe { cur_ptr.add(self.memory_map_len as usize) };

        MemoryEntryIter {
            cur_ptr,
            region_end,
        }
    }
}

#[repr(C, packed)]
#[derive(Clone, Copy, Debug)]
struct VbeTable {
    control_info: u32,
    mode_info: u32,
    mode: u16,
    interface_seg: u16,
    interface_off: u16,
    interface_len: u16,
}

#[repr(C, packed)]
#[derive(Clone, Copy, Debug)]
struct FramebufferTable {
    addr: u64,
    pitch: u32,
    width: u32,
    height: u32,
    bpp: u8,
    typ: u8,
    color_info: [u8; 6],
}

struct MemoryEntry {
    ptr: *const u8,
}

impl MemoryEntry {
    fn size(&self) -> u32 {
        unsafe { self.ptr.cast::<u32>().read_unaligned() }
    }

    fn base_addr(&self) -> u64 {
        unsafe { self.ptr.byte_add(4).cast::<u64>().read_unaligned() }
    }

    fn length(&self) -> u64 {
        unsafe { self.ptr.byte_add(12).cast::<u64>().read_unaligned() }
    }

    fn memory_type(&self) -> MemoryRegionType {
        let typ_val = unsafe { self.ptr.add(20).cast::<u8>().read_unaligned() };

        match typ_val {
            1 => MemoryRegionType::Usable,
            2 => MemoryRegionType::Reserved,
            3 => MemoryRegionType::Reclaimable,
            4 => MemoryRegionType::NonVolatileSleep,
            5 => MemoryRegionType::BadMemory,
            _ => MemoryRegionType::Reserved,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct MemoryEntryIter {
    cur_ptr: *const u8,
    region_end: *const u8,
}

impl Iterator for MemoryEntryIter {
    type Item = MemoryEntry;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cur_ptr >= self.region_end {
            return None;
        }

        let entry = MemoryEntry { ptr: self.cur_ptr };

        let entry_size = entry.size() as usize + 4;
        unsafe { self.cur_ptr = self.cur_ptr.add(entry_size) };

        Some(entry)
    }
}

unsafe fn parse_as_cstr<'a>(ptr: u32) -> Option<&'a str> {
    if ptr == 0 {
        return None;
    }

    let name_ptr = paddr_to_vaddr(ptr as usize) as *const core::ffi::c_char;
    let name_cstr = unsafe { core::ffi::CStr::from_ptr(name_ptr) };

    name_cstr.to_str().ok()
}

impl ToEarlyBootInfo for MultibootLegacyInfo {
    fn bootloader_name(&self) -> &'static str {
        unsafe { parse_as_cstr(self.boot_loader_name) }.unwrap_or("Unknown Multiboot Loader")
    }

    fn kernel_commandline(&self) -> Option<&'static str> {
        unsafe { parse_as_cstr(self.cmdline) }
    }

    fn initramfs(&self) -> Option<&'static [u8]> {
        if self.mods_count == 0 {
            return None;
        }

        let mods_addr = paddr_to_vaddr(self.mods_addr as usize) as *const u32;
        let (start, end) = unsafe { (*mods_addr, *mods_addr.add(1)) };

        let ptr = paddr_to_vaddr(start as usize) as *const u8;
        let len = (end - start) as usize;

        Some(unsafe { core::slice::from_raw_parts(ptr, len) })
    }

    fn acpi_arg(&self) -> BootloaderAcpiArg {
        BootloaderAcpiArg::ScanBios
    }

    fn framebuffer_arg(&self) -> Option<BootloaderFramebufferArg> {
        if self.framebuffer_table.addr == 0 {
            return None;
        }

        Some(BootloaderFramebufferArg {
            address: self.framebuffer_table.addr as usize,
            width: self.framebuffer_table.width as usize,
            height: self.framebuffer_table.height as usize,
            bpp: self.framebuffer_table.bpp as usize,
        })
    }

    fn memory_regions(
        &self,
        initramfs: Option<&'static [u8]>,
        kernel_cmdline: Option<&'static str>,
        framebuffer_arg: Option<BootloaderFramebufferArg>,
    ) -> MemoryRegionArray {
        let mut regions = MemoryRegionArray::new();

        let acpi_root_table_address = super::find_acpi_root_table_address();

        for entry in self.get_memory_map() {
            let base = entry.base_addr().try_into().unwrap();
            let len = entry.length().try_into().unwrap();
            let typ = super::effective_region_type(
                entry.memory_type(),
                base,
                len,
                acpi_root_table_address,
            );

            regions.push(MemoryRegion::new(base, len, typ)).unwrap();
        }

        if let Some(bootloader_name) = unsafe { parse_as_cstr(self.boot_loader_name) } {
            regions
                .push(MemoryRegion::module(bootloader_name.as_bytes()))
                .unwrap();
        }

        super::finish_memory_regions(regions, framebuffer_arg, initramfs, kernel_cmdline)
    }
}

#[unsafe(no_mangle)]
unsafe extern "sysv64" fn __multiboot_entry(boot_magic: u32, boot_params: u64) -> ! {
    assert_eq!(boot_magic, MULTIBOOT_ENTRY_MAGIC);
    let mb1_info =
        unsafe { &*(paddr_to_vaddr(boot_params as usize) as *const MultibootLegacyInfo) };

    use crate::boot::{EARLY_INFO, start_kernel};

    EARLY_INFO.call_once(|| mb1_info.to_early_boot_info());

    unsafe { start_kernel() };
}
