use core::sync::atomic::{AtomicU32, Ordering};

use crate::{cpu::CpuId, task::Task};

#[derive(Debug)]
pub struct TaskScheduleInfo {
    pub cpu: AtomicCpuId,
}

#[derive(Debug)]
pub struct AtomicCpuId(AtomicU32);

impl AtomicCpuId {
    const NONE: u32 = u32::MAX;

    pub fn set_if_is_none(&self, cpu_id: CpuId) -> Result<(), CpuId> {
        self.0
            .compare_exchange(
                Self::NONE,
                cpu_id.into(),
                Ordering::Relaxed,
                Ordering::Relaxed,
            )
            .map(|_| ())
            .map_err(|prev| (prev as usize).try_into().unwrap())
    }

    pub fn set_anyway(&self, cpu_id: CpuId) {
        self.0.store(cpu_id.into(), Ordering::Relaxed);
    }

    pub fn set_to_none(&self) {
        self.0.store(Self::NONE, Ordering::Relaxed);
    }

    pub fn get(&self) -> Option<CpuId> {
        let val = self.0.load(Ordering::Relaxed);
        if val == Self::NONE {
            None
        } else {
            Some((val as usize).try_into().ok()?)
        }
    }
}

impl Default for AtomicCpuId {
    fn default() -> Self {
        Self(AtomicU32::new(Self::NONE))
    }
}

impl CommonSchedInfo for Task {
    fn cpu(&self) -> &AtomicCpuId {
        &self.schedule_info().cpu
    }
}

pub trait CommonSchedInfo {
    fn cpu(&self) -> &AtomicCpuId;
}
