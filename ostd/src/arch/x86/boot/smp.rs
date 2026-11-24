//! Multiprocessor Boot Support
//!
//! The MP initialization protocol defines two classes of processors:
//! the bootstrap processor (BSP) and the application processors (APs).
//! Following a power-up or RESET of an MP system, system hardware dynamically
//! selects one of the processors on the system bus as the BSP. The remaining
//! processors are designated as APs.
//!
//! The BSP executes the BIOS's boot-strap code to configure the APIC environment,
//! sets up system-wide data structures. Up to now, BSP has completed most of the
//! initialization of the OS, but APs has not been awakened.
//!
//! Following a power-up or reset, the APs complete a minimal self-configuration,
//! then wait for a startup signal (a SIPI message) from the BSP processor.
//!
//! The wake-up of AP follows SNIT-SIPI-SIPI IPI sequence:
//!  - Broadcast INIT IPI (Initialize the APs to the wait-for-SIPI state)
//!  - Wait
//!  - Broadcast De-assert INIT IPI (Only older processors need this step)
//!  - Wait
//!  - Broadcast SIPI IPI (APs exits the wait-for-SIPI state and starts executing code)
//!  - Wait
//!  - Broadcast SIPI IPI (If an AP fails to start)
//!
//! This sequence does not need to be strictly followed, and there may be
//! different considerations in different systems.

use acpi::madt::MadtEntry;

use crate::{
    arch::{
        if_tdx_enabled,
        kernel::{
            acpi::get_acpi_tables,
            apic::{
                self, Apic, ApicId, DeliveryMode, DeliveryStatus, DestinationMode,
                DestinationShorthand, Icr, Level, TriggerMode,
            },
        },
    },
    boot::{
        memory_region::{MemoryRegion, MemoryRegionType},
        smp::PerApRawInfo,
    },
    mm::{PAGE_SIZE, Paddr, paddr_to_vaddr},
    task::disable_preempt,
};

/// Counts the number of processors.
///
/// This function needs to be called after the OS initializes the ACPI table.
pub(crate) fn count_processors() -> Option<u32> {
    let acpi_tables = get_acpi_tables()?;
    let madt_table = acpi_tables.find_table::<acpi::madt::Madt>().ok()?;

    fn is_usable(flags: u32) -> bool {
        const ENABLED: u32 = 0b01;
        const ONLINE_CAPABLE: u32 = 0b10;

        (flags & ENABLED) != 0 || (flags & ONLINE_CAPABLE) != 0
    }

    let is_dup_apic = |id: u32| -> bool {
        if madt_table.get().entries().any(|e| {
            matches!(e, MadtEntry::LocalX2Apic(e)
                if e.x2apic_id == id && is_usable(e.flags))
        }) {
            crate::warn!(
                "Firmware bug: In MADT, APIC ID {} is also listed as an x2APIC ID",
                id,
            );
            true
        } else {
            false
        }
    };

    let local_apic_counts = madt_table
        .get()
        .entries()
        .filter(|e| match e {
            MadtEntry::LocalX2Apic(entry) => {
                crate::debug!("Found a local x2APIC entry in MADT: {:?}", entry);
                is_usable(entry.flags)
            }
            MadtEntry::LocalApic(entry) => {
                crate::debug!("Found a local APIC entry in MADT: {:?}", entry);
                is_usable(entry.flags) && !is_dup_apic(entry.apic_id as u32)
            }
            _ => false,
        })
        .count();

    Some(local_apic_counts as u32)
}

/// Brings up all application processors.
///
/// # Safety
///
/// The caller must ensure that
/// 1. we're in the boot context of the BSP,
/// 2. all APs have not yet been booted, and
/// 3. the arguments are valid to boot APs.
pub(crate) unsafe fn bringup_all_aps(info_ptr: *const PerApRawInfo, pt_ptr: Paddr, num_cpus: u32) {
    unsafe {
        copy_ap_boot_code();
        fill_boot_info_ptr(info_ptr);
        fill_boot_pt_ptr(pt_ptr);
    }

    if_tdx_enabled!({
        unsafe { wake_up_aps_via_mailbox(num_cpus) };
    } else {
        unsafe { send_boot_ipis() };
    });
}

/// This is where the linker load the symbols in the `.ap_boot` section.
/// The BSP would copy the AP boot code to this address.
const AP_BOOT_START_PA: usize = 0x8000;

/// The size of the AP boot code (the `.ap_boot` section).
fn ap_boot_code_size() -> usize {
    __ap_boot_end as *const () as usize - __ap_boot_start as *const () as usize
}

pub(super) fn reclaimable_memory_region() -> MemoryRegion {
    MemoryRegion::new(
        AP_BOOT_START_PA,
        ap_boot_code_size(),
        MemoryRegionType::Reclaimable,
    )
}

/// # Safety
///
/// The caller must ensure the memory region to be filled with AP boot code is valid to write.
unsafe fn copy_ap_boot_code() {
    let ap_boot_start = __ap_boot_start as *const () as *const u8;
    let len = __ap_boot_end as *const () as usize - __ap_boot_start as *const () as usize;

    unsafe {
        core::ptr::copy_nonoverlapping(
            ap_boot_start,
            paddr_to_vaddr(AP_BOOT_START_PA) as *mut u8,
            len,
        );
    }
}

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

/// # Safety
///
/// This function writes to the static mutable variable `__boot_page_table_pointer`.
/// The caller must ensure exclusive access to this variable.
unsafe fn fill_boot_pt_ptr(pt_ptr: Paddr) {
    unsafe extern "C" {
        static mut __boot_page_table_pointer: u32;
    }

    let pt_ptr32 = pt_ptr.try_into().unwrap();

    unsafe {
        *(paddr_to_vaddr(&raw mut __boot_page_table_pointer as Paddr) as *mut u32) = pt_ptr32
    };
}

unsafe extern "C" {
    fn __ap_boot_start();
    fn __ap_boot_end();
}

/// Wakes up all application processors via the ACPI multiprocessor mailbox structure.
///
/// # Safety
///
/// The safety preconditions are the same as [`send_boot_ipis`].
#[cfg(feature = "cvm_guest")]
unsafe fn wake_up_aps_via_mailbox(num_cpus: u32) {
    use acpi::platform::wakeup_aps;

    use crate::arch::kernel::acpi::AcpiMemoryHandler;

    unsafe extern "C" {
        fn ap_boot_from_real_mode();
        fn ap_boot_from_long_mode();
    }

    let offset =
        ap_boot_from_long_mode as *const () as usize - ap_boot_from_real_mode as *const () as usize;

    let acpi_tables = get_acpi_tables().unwrap();
    for ap_num in 1..num_cpus {
        wakeup_aps(
            acpi_tables,
            AcpiMemoryHandler {},
            ap_num,
            (AP_BOOT_START_PA + offset) as u64,
            1000,
        )
        .unwrap();
    }
}

/// Sends IPIs to notify all application processors to boot.
///
/// Follow the INIT-SIPI-SIPI IPI sequence.
/// Here, we don't check whether there is an AP that failed to start,
/// but send the second SIPI directly (checking whether each core is
/// started successfully one by one will bring extra overhead). For
/// APs that have been started, this signal will not bring any cost.
///
/// # Safety
///
/// The caller must ensure that all application processors can be
/// safely booted by ensuring that:
/// 1. We're in the boot context of the BSP and all APs have not yet
///    been booted.
/// 2. We've properly prepared all the resources for the application
///    processors to boot successfully (e.g., each AP's page table
///    and stack).
unsafe fn send_boot_ipis() {
    let preempt_guard = disable_preempt();
    let apic = apic::get_or_init(&preempt_guard as _);

    unsafe {
        send_init_to_all_aps(apic);
        spin_wait_cycles(100_000_000);

        send_init_deassert(apic);
        spin_wait_cycles(20_000_000);

        send_startup_to_all_aps(apic);
        spin_wait_cycles(20_000_000);

        send_startup_to_all_aps(apic);
        spin_wait_cycles(20_000_000);
    }
}

/// # Safety
///
/// The caller should ensure it's valid to send STARTUP IPIs to all CPUs excluding self.
unsafe fn send_startup_to_all_aps(apic: &dyn Apic) {
    let icr = Icr::new(
        ApicId::from(0),
        DestinationShorthand::AllExcludingSelf,
        TriggerMode::Edge,
        Level::Assert,
        DeliveryStatus::Idle,
        DestinationMode::Physical,
        DeliveryMode::StartUp,
        (AP_BOOT_START_PA / PAGE_SIZE) as u8,
    );

    unsafe { apic.send_ipi(icr) }
}

/// # Safety
///
/// The caller should ensure it's valid to send INIT IPIs to all CPUs excluding self.
unsafe fn send_init_to_all_aps(apic: &dyn Apic) {
    let icr = Icr::new(
        ApicId::from(0),
        DestinationShorthand::AllExcludingSelf,
        TriggerMode::Level,
        Level::Assert,
        DeliveryStatus::Idle,
        DestinationMode::Physical,
        DeliveryMode::Init,
        0,
    );

    unsafe { apic.send_ipi(icr) };
}

/// # Safety
///
/// The caller should ensure it's valid to deassert INIT IPIs for all CPUs excluding self.
unsafe fn send_init_deassert(apic: &dyn Apic) {
    let icr = Icr::new(
        ApicId::from(0),
        DestinationShorthand::AllIncludingSelf,
        TriggerMode::Level,
        Level::Deassert,
        DeliveryStatus::Idle,
        DestinationMode::Physical,
        DeliveryMode::Init,
        0,
    );

    unsafe { apic.send_ipi(icr) };
}

/// Spin wait approximately `c` cycles.
///
/// Since the timer requires CPU local storage to be initialized, we
/// can only wait by spinning.
fn spin_wait_cycles(c: u64) {
    fn duration(from: u64, to: u64) -> u64 {
        if to >= from {
            to - from
        } else {
            u64::MAX - from + to
        }
    }

    let start = crate::arch::read_tsc();
    while duration(start, crate::arch::read_tsc()) < c {
        core::hint::spin_loop();
    }
}
