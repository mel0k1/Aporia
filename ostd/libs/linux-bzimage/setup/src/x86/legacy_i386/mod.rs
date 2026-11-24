use core::arch::{asm, global_asm};

use linux_boot_params::BootParams;

pub(super) mod alloc;
mod efi;

global_asm!(include_str!("setup.S"));

const ASTER_ENTRY_POINT: *const () = 0x8001000 as _;

#[unsafe(export_name = "main_legacy32")]
extern "cdecl" fn main_legacy32(boot_params_ptr: *mut BootParams) -> ! {
    crate::println!(
        "[setup] Loaded with offset {:#x}",
        crate::x86::image_load_offset(),
    );

    let boot_params = unsafe { &mut *boot_params_ptr };

    efi::fill_acpi_rsdp_addr(boot_params);

    unsafe { alloc::init(boot_params) };

    crate::println!("[setup] Loading the payload as an ELF file");
    crate::loader::load_elf(crate::x86::payload());

    crate::println!(
        "[setup] Entering the Aporia entry point at {:p}",
        ASTER_ENTRY_POINT,
    );
    unsafe { call_aster_entrypoint(ASTER_ENTRY_POINT, boot_params_ptr) };
}

unsafe fn call_aster_entrypoint(entrypoint: *const (), boot_params_ptr: *mut BootParams) -> ! {
    unsafe {
        asm!(
            "mov esi, {1}",
            "mov eax, {0}",
            "jmp eax",
            in(reg) entrypoint,
            in(reg) boot_params_ptr,
            options(noreturn),
        )
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    crate::println!("[PANIC]: {}", info);

    loop {
        unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)) };
    }
}
