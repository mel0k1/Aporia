use core::{ffi::CStr, mem::MaybeUninit};

use boot::{AllocateType, open_protocol_exclusive};
use linux_boot_params::BootParams;
use uefi::{mem::memory_map::MemoryMap, prelude::*, proto::BootPolicy};
use uefi_raw::table::system::SystemTable;

use super::decoder::decode_payload;
use crate::x86::amd64_efi::alloc::alloc_pages;

pub(super) const PAGE_SIZE: u64 = 4096;

#[unsafe(no_mangle)]
unsafe extern "sysv64" fn main_efi_common64(
    handle: Handle,
    system_table: *const SystemTable,
    boot_params_ptr: *mut BootParams,
) -> ! {
    unsafe {
        boot::set_image_handle(handle);
        uefi::table::set_system_table(system_table);
    }

    uefi::helpers::init().unwrap();

    let boot_params = if boot_params_ptr.is_null() {
        allocate_boot_params()
    } else {
        unsafe { &mut *boot_params_ptr }
    };

    efi_phase_boot(boot_params);

    unsafe { efi_phase_runtime(boot_params) };
}

fn allocate_boot_params() -> &'static mut BootParams {
    let boot_params = {
        let bytes = alloc_pages(AllocateType::AnyPages, size_of::<BootParams>());
        bytes.write_filled(0);

        unsafe { &mut *bytes.as_mut_ptr().cast::<BootParams>() }
    };

    boot_params.hdr.header = linux_boot_params::LINUX_BOOT_HEADER_MAGIC;

    boot_params
}

fn efi_phase_boot(boot_params: &mut BootParams) {
    uefi::println!(
        "[EFI stub] Loaded with offset {:#x}",
        crate::x86::image_load_offset(),
    );

    if boot_params.hdr.cmd_line_ptr == 0
        && boot_params.ext_cmd_line_ptr == 0
        && let Some(cmdline) = load_cmdline()
    {
        boot_params.hdr.cmd_line_ptr = cmdline.as_ptr().addr().try_into().unwrap();
        boot_params.ext_cmd_line_ptr = 0;
        boot_params.hdr.cmdline_size = (cmdline.count_bytes() + 1).try_into().unwrap();
    }

    if boot_params.hdr.ramdisk_image == 0
        && boot_params.ext_ramdisk_image == 0
        && let Some(initrd) = load_initrd()
    {
        boot_params.hdr.ramdisk_image = initrd.as_ptr().addr().try_into().unwrap();
        boot_params.ext_ramdisk_image = 0;
        boot_params.hdr.ramdisk_size = initrd.len().try_into().unwrap();
        boot_params.ext_ramdisk_size = 0;
    }

    if boot_params.acpi_rsdp_addr == 0 {
        boot_params.acpi_rsdp_addr =
            find_rsdp_addr().expect("ACPI RSDP address is not available") as usize as u64;
    }

    if boot_params.screen_info.lfb_base == 0 && boot_params.screen_info.ext_lfb_base == 0 {
        fill_screen_info(&mut boot_params.screen_info);
    }

    uefi::println!("[EFI stub] Decoding the kernel payload");
    let kernel = decode_payload(crate::x86::payload());
    uefi::println!("[EFI stub] Loading the payload as an ELF file");
    crate::loader::load_elf(&kernel);
}

fn load_cmdline() -> Option<&'static CStr> {
    uefi::println!("[EFI stub] Loading the cmdline");

    let loaded_image =
        open_protocol_exclusive::<uefi::proto::loaded_image::LoadedImage>(boot::image_handle())
            .unwrap();

    let Some(load_options) = loaded_image.load_options_as_bytes() else {
        uefi::println!("[EFI stub] Warning: No cmdline is available!");
        return None;
    };

    if !load_options.len().is_multiple_of(2)
        || load_options.iter().skip(1).step_by(2).any(|c| *c != 0)
    {
        uefi::println!("[EFI stub] Warning: The cmdline contains non-ASCII characters!");
        return None;
    }

    let cmdline_bytes = alloc_pages(
        AllocateType::MaxAddress(u32::MAX as u64),
        load_options.len() / 2 + 1,
    );
    for i in 0..load_options.len() / 2 {
        cmdline_bytes[i].write(load_options[i * 2]);
    }
    cmdline_bytes[load_options.len() / 2].write(0);

    let cmdline_str =
        CStr::from_bytes_until_nul(unsafe { cmdline_bytes.assume_init_ref() }).unwrap();

    uefi::println!("[EFI stub] Loaded the cmdline: {:?}", cmdline_str);

    Some(cmdline_str)
}

fn load_initrd() -> Option<&'static [u8]> {
    uefi::println!("[EFI stub] Loading the initrd");

    #[repr(transparent)]
    #[uefi::proto::unsafe_protocol(uefi_raw::protocol::media::LoadFile2Protocol::GUID)]
    struct LoadFile2(uefi_raw::protocol::media::LoadFile2Protocol);

    let mut device_path_buf = [MaybeUninit::uninit(); 20 + 4];
    let mut device_path = {
        use uefi::proto::device_path::build;
        build::DevicePathBuilder::with_buf(&mut device_path_buf)
            .push(&build::media::Vendor {
                vendor_guid: uefi::guid!("5568e427-68fc-4f3d-ac74-ca555231cc68"),
                vendor_defined_data: &[],
            })
            .unwrap()
            .finalize()
            .unwrap()
    };

    let Ok(handle) = boot::locate_device_path::<LoadFile2>(&mut device_path) else {
        uefi::println!("[EFI stub] Warning: Failed to locate the initrd device!");
        return None;
    };

    let Ok(mut load_file2) = open_protocol_exclusive::<LoadFile2>(handle) else {
        uefi::println!("[EFI stub] Warning: Failed to open the initrd protocol!");
        return None;
    };

    let mut size = 0;

    let status = unsafe {
        (load_file2.0.load_file)(
            &mut load_file2.0,
            device_path.as_ffi_ptr().cast(),
            BootPolicy::ExactMatch.into(),
            &mut size,
            core::ptr::null_mut(),
        )
    };
    if status != Status::BUFFER_TOO_SMALL {
        uefi::println!("[EFI stub] Warning: Failed to get the initrd size!");
        return None;
    }

    let initrd = alloc_pages(AllocateType::MaxAddress(u32::MAX as u64), size);

    let status = unsafe {
        (load_file2.0.load_file)(
            &mut load_file2.0,
            device_path.as_ffi_ptr().cast(),
            BootPolicy::ExactMatch.into(),
            &mut size,
            initrd.as_mut_ptr().cast(),
        )
    };
    if status.is_error() {
        uefi::println!("[EFI stub] Warning: Failed to load the initrd!");
        return None;
    }
    assert_eq!(
        size,
        initrd.len(),
        "the initrd size has changed between two EFI calls"
    );

    uefi::println!(
        "[EFI stub] Loaded the initrd: addr={:#x}, size={:#x}",
        initrd.as_ptr().addr(),
        initrd.len()
    );

    Some(unsafe { initrd.assume_init_ref() })
}

fn find_rsdp_addr() -> Option<*const ()> {
    use uefi::table::cfg::ConfigTableEntry;

    for acpi_guid in [ConfigTableEntry::ACPI2_GUID, ConfigTableEntry::ACPI_GUID] {
        if let Some(rsdp_addr) = system::with_config_table(|table| {
            table
                .iter()
                .find(|entry| entry.guid == acpi_guid)
                .map(|entry| entry.address.cast::<()>())
        }) {
            uefi::println!("[EFI stub] Found the ACPI RSDP at {:p}", rsdp_addr);
            return Some(rsdp_addr);
        }
    }

    uefi::println!("[EFI stub] Warning: Failed to find the ACPI RSDP address!");

    None
}

fn fill_screen_info(screen_info: &mut linux_boot_params::ScreenInfo) {
    use uefi::{
        boot::{OpenProtocolAttributes, OpenProtocolParams, open_protocol},
        proto::console::gop::{GraphicsOutput, PixelFormat},
    };

    let Ok(handle) = boot::get_handle_for_protocol::<GraphicsOutput>() else {
        uefi::println!("[EFI stub] Warning: Failed to locate the graphics handle!");
        return;
    };

    let Ok(mut protocol) = (unsafe {
        open_protocol::<GraphicsOutput>(
            OpenProtocolParams {
                handle,
                agent: boot::image_handle(),
                controller: None,
            },
            OpenProtocolAttributes::GetProtocol,
        )
    }) else {
        uefi::println!("[EFI stub] Warning: Failed to open the graphics protocol!");
        return;
    };

    if !matches!(
        protocol.current_mode_info().pixel_format(),
        PixelFormat::Rgb | PixelFormat::Bgr
    ) {
        uefi::println!(
            "[EFI stub] Warning: Ignored the framebuffer as the pixel format is not supported!"
        );
        return;
    }

    let addr = protocol.frame_buffer().as_mut_ptr().addr();
    let (width, height) = protocol.current_mode_info().resolution();

    screen_info.lfb_base = addr as u32;
    screen_info.ext_lfb_base = (addr >> 32) as u32;
    screen_info.lfb_width = width.try_into().unwrap();
    screen_info.lfb_height = height.try_into().unwrap();
    screen_info.lfb_depth = 32;

    uefi::println!(
        "[EFI stub] Found the framebuffer at {:#x} with {}x{} pixels",
        addr,
        width,
        height
    );
}

unsafe fn efi_phase_runtime(boot_params: &mut BootParams) -> ! {
    #[cfg(feature = "cvm_guest")]
    let unaccepted_memory_builder =
        super::unaccepted_memory::prepare_unaccepted_memory(boot_params);

    uefi::println!("[EFI stub] Exiting EFI boot services");

    let memory_map = unsafe { boot::exit_boot_services(Some(boot::MemoryType::LOADER_DATA)) };

    crate::println!(
        "[EFI stub] Processing {} memory map entries",
        memory_map.entries().len()
    );

    #[cfg(feature = "cvm_guest")]
    unsafe {
        super::unaccepted_memory::finish_unaccepted_memory(
            boot_params,
            unaccepted_memory_builder,
            &memory_map,
        );
    }

    populate_e820_from_memory_map(boot_params, &memory_map);

    crate::println!(
        "[EFI stub] Entering the Asterinas entry point at {:p}",
        super::ASTER_ENTRY_POINT,
    );

    unsafe { super::call_aster_entrypoint(super::ASTER_ENTRY_POINT, boot_params) }
}

fn populate_e820_from_memory_map<M: MemoryMap>(boot_params: &mut BootParams, memory_map: &M) {
    let e820_table = &mut boot_params.e820_table;
    let mut num_entries = 0usize;

    for entry in memory_map.entries() {
        #[cfg(feature = "debug_print")]
        crate::println!(
            "    [{:#x}] {:#x} (size={:#x}) {{flags={:#x}}}",
            entry.ty.0,
            entry.phys_start,
            entry.page_count,
            entry.att.bits()
        );

        let typ = parse_memory_type(entry.ty);

        if num_entries != 0 {
            let last_entry = &mut e820_table[num_entries - 1];
            let last_typ = last_entry.typ;
            if last_typ == typ && last_entry.addr + last_entry.size == entry.phys_start {
                last_entry.size += entry.page_count * PAGE_SIZE;
                continue;
            }
        }

        if num_entries >= e820_table.len() {
            crate::println!("[EFI stub] Warning: The number of E820 entries exceeded 128!");
            break;
        }

        e820_table[num_entries] = linux_boot_params::BootE820Entry {
            addr: entry.phys_start,
            size: entry.page_count * PAGE_SIZE,
            typ,
        };
        num_entries += 1;
    }
    boot_params.e820_entries = num_entries as u8;
}

fn parse_memory_type(mem_type: boot::MemoryType) -> linux_boot_params::E820Type {
    use linux_boot_params::E820Type;
    use uefi::boot::MemoryType;

    match mem_type {
        MemoryType::LOADER_CODE
        | MemoryType::LOADER_DATA
        | MemoryType::BOOT_SERVICES_CODE
        | MemoryType::BOOT_SERVICES_DATA
        | MemoryType::CONVENTIONAL => E820Type::RAM,

        MemoryType::PERSISTENT_MEMORY => E820Type::PMEM,
        MemoryType::ACPI_RECLAIM => E820Type::ACPI,
        MemoryType::ACPI_NON_VOLATILE => E820Type::NVS,
        MemoryType::UNUSABLE => E820Type::UNUSABLE,
        MemoryType::UNACCEPTED => {
            #[cfg(feature = "cvm_guest")]
            if tdx_guest::is_tdx_guest_early() {
                return E820Type::RAM;
            }

            crate::println!(
                "[EFI stub] Warning: UNACCEPTED memory is unsupported outside a confidential VM!"
            );
            E820Type::RESERVED
        }

        MemoryType::RESERVED
        | MemoryType::RUNTIME_SERVICES_CODE
        | MemoryType::RUNTIME_SERVICES_DATA
        | MemoryType::MMIO
        | MemoryType::MMIO_PORT_SPACE => E820Type::RESERVED,
        _ => E820Type::RESERVED,
    }
}
