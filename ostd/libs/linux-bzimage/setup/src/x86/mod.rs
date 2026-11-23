use core::arch::global_asm;

cfg_select! {
    target_arch = "x86_64" => {
        mod amd64_efi;

        pub(crate) use amd64_efi::alloc::alloc_at;

        const CFG_TARGET_ARCH_X86_64: usize = 1;
    }
    target_arch = "x86" => {
        mod legacy_i386;

        pub(crate) use legacy_i386::alloc::alloc_at;

        const CFG_TARGET_ARCH_X86_64: usize = 0;
    }
    _ => {
        compile_error!("unsupported target");
    }
}

global_asm!(
    include_str!("header.S"),
    CFG_TARGET_ARCH_X86_64 = const CFG_TARGET_ARCH_X86_64,
);

pub(crate) fn image_load_offset() -> isize {
    const CODE32_START: isize = 0x100000;

    unsafe extern "C" {
        fn entry_legacy32();
    }

    (entry_legacy32 as *const () as usize as isize) - CODE32_START
}

global_asm!(
    ".section \".payload\", \"a\"",
    concat!(".incbin \"", env!("PAYLOAD_FILE"), "\""),
);

fn payload() -> &'static [u8] {
    unsafe extern "C" {
        fn __payload_start();
        fn __payload_end();
    }

    unsafe {
        core::slice::from_raw_parts(
            __payload_start as *const u8,
            __payload_end as *const () as usize - __payload_start as *const () as usize,
        )
    }
}
