//! Virtio over MMIO

use bus::MmioBus;
use ostd::{debug, io::IoMem, mm::HasPaddr, sync::SpinLock};

use crate::transport::mmio::bus::common_device::{mmio_check_magic, mmio_read_device_id};

#[cfg_attr(target_arch = "x86_64", path = "arch/x86.rs")]
#[cfg_attr(target_arch = "riscv64", path = "arch/riscv.rs")]
#[cfg_attr(target_arch = "loongarch64", path = "arch/loongarch.rs")]
#[cfg_attr(target_arch = "aarch64", path = "arch/arm.rs")]
mod arch;

#[expect(clippy::module_inception)]
pub(super) mod bus;
pub(super) mod common_device;

/// The MMIO bus instance.
pub(super) static MMIO_BUS: SpinLock<MmioBus> = SpinLock::new(MmioBus::new());

pub(super) fn init() {
    #[cfg(target_arch = "x86_64")]
    ostd::if_tdx_enabled!({




    } else {
        arch::probe_for_device();
    });
    #[cfg(not(target_arch = "x86_64"))]
    arch::probe_for_device();
}

/// Validates a potential VirtIO-MMIO device.
fn validate_mmio_device(io_mem: &IoMem) -> Result<(), MmioValidateError> {
    if !mmio_check_magic(io_mem) {
        debug!(
            "Abort MMIO detection at {:#x} because the magic number does not match",
            io_mem.paddr()
        );
        return Err(MmioValidateError::MagicMismatch);
    }

    match mmio_read_device_id(io_mem) {
        Err(_) | Ok(0) => Err(MmioValidateError::NoDevice),
        Ok(_) => Ok(()),
    }
}

#[derive(Clone, Copy, Debug)]
enum MmioValidateError {
    /// Not a VirtIO-MMIO slot.
    MagicMismatch,
    /// No device present.
    NoDevice,
}
