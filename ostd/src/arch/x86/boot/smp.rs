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

const AP_BOOT_START_PA: usize = 0x8000;

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

unsafe fn fill_boot_info_ptr(info_ptr: *const PerApRawInfo) {
    unsafe extern "C" {
        static mut __ap_boot_info_array_pointer: *const PerApRawInfo;
    }

    unsafe {
        __ap_boot_info_array_pointer = info_ptr;
    }
}

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
