//! Architecture kernel module.

pub(super) mod acpi;
pub(super) mod apic;
pub(super) mod kvm_clock;
pub(super) mod tsc;

pub use acpi::{ACPI_INFO, AcpiInfo};
pub use kvm_clock::{PvclockTimeSnapshot, read_pvclock_time_snapshot, read_wall_clock_epoch};
