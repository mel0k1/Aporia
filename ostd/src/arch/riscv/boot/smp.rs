//! Multiprocessor Boot Support

use core::arch::global_asm;

use crate::{
    boot::smp::{PerApRawInfo, ap_early_entry},
    cpu_local_cell,
    mm::{Paddr, kspace::kernel_loaded_offset},
};

global_asm!(include_str!("ap_boot.S"));

pub(crate) fn count_processors() -> Option<u32> {
    let mut hart_count = 0;

    for_each_hart_id(|_| hart_count += 1);

    if hart_count == 0 {
        None
    } else {
        Some(hart_count)
    }
}

/// Brings up all application processors.
///
/// Following the x86 naming, all the harts that are not the bootstrapping hart
/// are "application processors".
///
/// # Safety
///
/// The caller must ensure that
///  1. we're in the boot context of the BSP,
///  2. all APs have not yet been booted, and
///  3. the arguments are valid to boot APs.
pub(crate) unsafe fn bringup_all_aps(info_ptr: *const PerApRawInfo, pt_ptr: Paddr, num_cpus: u32) {
    if num_cpus <= 1 {
        return;
    }

    unsafe {
        fill_boot_info_ptr(info_ptr);
        fill_boot_page_table_ptr(pt_ptr);
    }

    let bsp_id = get_bootstrap_hart_id();

    crate::info!("Bootstrapping hart is {}, booting all other harts", bsp_id);

    for_each_hart_id(|hart_id| {
        if hart_id != bsp_id {
            unsafe { bringup_ap(hart_id) };
        }
    });
}

fn for_each_hart_id(mut f: impl FnMut(u32)) {
    let Some(device_tree) = super::DEVICE_TREE.get() else {
        f(get_bootstrap_hart_id());
        return;
    };

    device_tree.cpus().for_each(|cpu_node| {
        if let Some(device_type) = cpu_node.property("device_type") {
            if device_type.as_str() == Some("cpu")
                && cpu_node.property("mmu-type").is_some()
                && let Some(reg) = cpu_node.property("reg")
            {
                f(reg.as_usize().unwrap() as u32);
            }
        }
    })
}

/// # Safety
///
/// The caller must ensure that
///  1. we're in the boot context of the BSP,
///  2. the resources for APs' boot (e.g., stacks and the page table) are set
///     up correctly, and
///  3. the `hart_id` hart hasn't booted.
unsafe fn bringup_ap(hart_id: u32) {
    crate::info!("Starting hart {}", hart_id);

    let result = sbi_rt::hart_start(hart_id as usize, get_ap_boot_start_addr(), 0);

    if result.error == 0 {
        crate::debug!("Successfully started hart {}", hart_id);
    } else {
        crate::error!(
            "Failed to start hart {}: error code {}",
            hart_id,
            result.error
        );
    }
}

/// Fills the AP boot info array pointer.
///
/// # Safety
///
/// This function writes to the static mutable variable `__ap_boot_info_array_pointer`.
/// The caller must ensure exclusive access to this variable.
unsafe fn fill_boot_info_ptr(info_ptr: *const PerApRawInfo) {
    unsafe extern "C" {
        static mut __ap_boot_info_array_pointer: *const PerApRawInfo;
    }

    unsafe {
        __ap_boot_info_array_pointer = info_ptr;
    }
}

/// Fills the AP boot page table pointer.
///
/// # Safety
///
/// This function writes to the static mutable variable `__ap_boot_page_table_pointer`.
/// The caller must ensure exclusive access to this variable.
unsafe fn fill_boot_page_table_ptr(pt_ptr: Paddr) {
    unsafe extern "C" {
        static mut __ap_boot_page_table_pointer: Paddr;
    }

    unsafe {
        __ap_boot_page_table_pointer = pt_ptr;
    }
}

fn get_ap_boot_start_addr() -> Paddr {
    unsafe extern "C" {
        fn __ap_boot_start();
    }

    __ap_boot_start as *const () as Paddr - kernel_loaded_offset()
}

fn get_bootstrap_hart_id() -> u32 {
    unsafe { super::BOOTSTRAP_HART_ID }
}

pub(in crate::arch) fn get_current_hart_id() -> u32 {
    let id = AP_CURRENT_HART_ID.load();
    if id == u32::MAX {
        get_bootstrap_hart_id()
    } else {
        id
    }
}

cpu_local_cell! {
    static AP_CURRENT_HART_ID: u32 = u32::MAX;
}

/// # Safety
///
/// - This function must be called only once on each AP at a proper timing in the AP's boot
///   assembly code.
/// - The caller must follow C calling conventions and put the right arguments in registers.
#[unsafe(no_mangle)]
unsafe extern "C" fn riscv_ap_early_entry(cpu_id: u32, hart_id: u32) -> ! {
    AP_CURRENT_HART_ID.store(hart_id);

    unsafe { ap_early_entry(cpu_id) }
}
