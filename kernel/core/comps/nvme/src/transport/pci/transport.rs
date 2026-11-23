use alloc::sync::Arc;
use core::{
    fmt::Debug,
    ops::{Deref, DerefMut},
};

use aster_pci::{
    bus::{PciDevice, PciDeviceExt},
    cfg_space::{Bar, BarAccess},
    common_device::PciCommonDevice,
};
use ostd::{
    error, info,
    sync::{LocalIrqDisabled, SpinLock, SpinLockGuard},
};

use crate::{
    msix::NvmeMsixManager,
    nvme_queue::QUEUE_NUM,
    nvme_regs::{NVME_BAR0_FIXED_REGS_END, NvmeDoorbellRegs, NvmeRegs32, NvmeRegs64},
};

pub(crate) struct NvmePciTransport {
    inner: NvmePciTransportInner,
    config_bar: BarAccess,
}

pub(crate) struct NvmePciTransportInner {
    common_device: Arc<PciDevice>,
    msix_manager: NvmeMsixManager,
}

impl Debug for NvmePciTransport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NvmePciTransport")
            .field("common_device", &self.inner.common_device)
            .finish_non_exhaustive()
    }
}

impl NvmePciTransport {
    pub(super) fn new(common_device: Arc<PciDevice>) -> Result<Self, aster_device::common::Error> {
        let Some(config_bar) = Self::check_and_acquire_bar0(&common_device) else {
            error!("BAR0 is unusable: missing, not MMIO, map failed, or too small");
            return Err(aster_device::common::Error::InvalidValue);
        };

        let Some(msix_manager) = Self::init_msix(&common_device) else {
            error!("MSI-X capability missing or MSI-X setup failed");
            return Err(aster_device::common::Error::ResourceUnavailable);
        };

        Ok(Self {
            inner: NvmePciTransportInner {
                common_device,
                msix_manager,
            },
            config_bar,
        })
    }

    fn check_and_acquire_bar0(device: &PciCommonDevice) -> Option<BarAccess> {
        let bar0 = device.bar_manager().bar(0)?;
        let bar_size = match bar0 {
            Bar::Memory(mem) => mem.size(),
            Bar::Io(_) => return None,
        };

        if bar_size < NVME_BAR0_FIXED_REGS_END {
            return None;
        }

        let config_bar = bar0.acquire().ok()?;
        let cap = RegAccess(&config_bar).read64(NvmeRegs64::Cap);
        let required_bar = Self::required_bar0_size_bytes(cap);
        if bar_size < required_bar {
            return None;
        }

        Some(config_bar)
    }

    fn required_bar0_size_bytes(cap: u64) -> u64 {
        let dstrd = ((cap >> NvmeRegs64::CAP_DSTRD_SHIFT) & NvmeRegs64::CAP_DSTRD_MASK) as u16;
        NVME_BAR0_FIXED_REGS_END
            .max(NvmeDoorbellRegs::Sqtdbl.offset(QUEUE_NUM as u16, dstrd) as u64)
    }

    fn init_msix(common_device: &Arc<PciDevice>) -> Option<NvmeMsixManager> {
        let msix_opt = common_device.acquire_msix_capability().ok()?;
        let msix_data = msix_opt?;
        let manager = NvmeMsixManager::new(msix_data)?;
        info!("MSI-X enabled with {} vectors", manager.table_size());
        Some(manager)
    }

    pub(crate) fn msix_manager_mut(&mut self) -> &mut NvmeMsixManager {
        self.inner.msix_manager_mut()
    }

    pub(crate) fn regs(&mut self) -> RegAccess<'_> {
        RegAccess(&self.config_bar)
    }

    pub(crate) fn dbregs(&self) -> DbregAccess<'_> {
        DbregAccess(&self.config_bar)
    }
}

impl NvmePciTransportInner {
    pub(crate) fn msix_manager_mut(&mut self) -> &mut NvmeMsixManager {
        &mut self.msix_manager
    }
}

pub(crate) struct NvmePciTransportLock {
    inner: SpinLock<NvmePciTransportInner, LocalIrqDisabled>,
    config_bar: BarAccess,
}

pub(crate) struct NvmePciTransportGuard<'a> {
    inner: SpinLockGuard<'a, NvmePciTransportInner, LocalIrqDisabled>,
}

impl Deref for NvmePciTransportGuard<'_> {
    type Target = NvmePciTransportInner;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for NvmePciTransportGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl NvmePciTransportLock {
    pub(crate) fn new(inner: NvmePciTransport) -> Self {
        Self {
            inner: SpinLock::new(inner.inner),
            config_bar: inner.config_bar,
        }
    }

    pub(crate) fn lock(&self) -> NvmePciTransportGuard<'_> {
        NvmePciTransportGuard {
            inner: self.inner.lock(),
        }
    }

    pub(crate) fn dbregs(&self) -> DbregAccess<'_> {
        DbregAccess(&self.config_bar)
    }
}

pub(crate) struct RegAccess<'a>(&'a BarAccess);

impl RegAccess<'_> {
    pub(crate) fn read32(&self, reg: NvmeRegs32) -> u32 {
        self.0.read_once(reg as usize).unwrap()
    }

    pub(crate) fn read64(&self, reg: NvmeRegs64) -> u64 {
        let base = reg as usize;
        let reg_low: u32 = self.0.read_once(base).unwrap();
        let reg_high: u32 = self.0.read_once(base + 4).unwrap();
        ((reg_high as u64) << 32) | (reg_low as u64)
    }

    pub(crate) fn write32(&self, reg: NvmeRegs32, val: u32) {
        self.0.write_once(reg as usize, val).unwrap();
    }

    pub(crate) fn write64(&self, reg: NvmeRegs64, val: u64) {
        let base = reg as usize;
        let val_low = (val & 0xFFFF_FFFF) as u32;
        let val_high = (val >> 32) as u32;
        self.0.write_once(base, val_low).unwrap();
        self.0.write_once(base + 4, val_high).unwrap();
    }
}

pub(crate) struct DbregAccess<'a>(&'a BarAccess);

impl DbregAccess<'_> {
    pub(crate) fn write_racy(&self, reg: NvmeDoorbellRegs, qid: u16, dstrd: u16, val: u32) {
        let offset = reg.offset(qid, dstrd);
        self.0.write_once(offset, val).unwrap();
    }
}
