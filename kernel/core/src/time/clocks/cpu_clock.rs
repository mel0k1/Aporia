use alloc::sync::Arc;
use core::time::Duration;

use ostd::{
    sync::{LocalIrqDisabled, SpinLock},
    timer::Jiffies,
};

use crate::time::Clock;

pub(crate) struct CpuClock {
    time: SpinLock<Jiffies, LocalIrqDisabled>,
}

pub(crate) struct ProfClock {
    user_clock: Arc<CpuClock>,
    kernel_clock: Arc<CpuClock>,
}

impl CpuClock {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            time: SpinLock::new(Jiffies::new(0)),
        })
    }

    pub(crate) fn add_jiffies(&self, jiffies: u64) {
        self.time.lock().add(jiffies);
    }

    pub(crate) fn read_jiffies(&self) -> Jiffies {
        *self.time.lock()
    }
}

impl Clock for CpuClock {
    fn read_time(&self) -> Duration {
        self.read_jiffies().as_duration()
    }
}

impl ProfClock {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            user_clock: CpuClock::new(),
            kernel_clock: CpuClock::new(),
        })
    }

    pub(crate) fn user_clock(&self) -> &Arc<CpuClock> {
        &self.user_clock
    }

    pub(crate) fn kernel_clock(&self) -> &Arc<CpuClock> {
        &self.kernel_clock
    }
}

impl Clock for ProfClock {
    fn read_time(&self) -> Duration {
        self.user_clock.read_time() + self.kernel_clock.read_time()
    }
}
