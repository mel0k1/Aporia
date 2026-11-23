use core::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use super::TIMER_FREQ;

#[derive(Clone, Copy, Debug)]
pub struct Jiffies(u64);

pub(crate) static ELAPSED: AtomicU64 = AtomicU64::new(0);

impl Jiffies {
    pub const MAX: Self = Self(u64::MAX);

    pub fn new(value: u64) -> Self {
        Self(value)
    }

    pub fn elapsed() -> Self {
        Self::new(ELAPSED.load(Ordering::Relaxed))
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }

    pub fn add(&mut self, jiffies: u64) {
        self.0 = self.0.saturating_add(jiffies);
    }

    pub fn as_duration(self) -> Duration {
        let secs = self.0 / TIMER_FREQ;
        let nanos = ((self.0 % TIMER_FREQ) * 1_000_000_000) / TIMER_FREQ;
        Duration::new(secs, nanos as u32)
    }
}

impl From<Jiffies> for Duration {
    fn from(value: Jiffies) -> Self {
        value.as_duration()
    }
}
