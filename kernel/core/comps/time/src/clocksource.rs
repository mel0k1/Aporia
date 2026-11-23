use alloc::sync::Arc;
use core::{cmp::max, ops::Add, time::Duration};

use aster_util::coeff::Coeff;
use ostd::sync::{LocalIrqDisabled, RwLock};

const NANOS_PER_SECOND: u32 = 1_000_000_000;

pub struct ClockSource {
    read_cycles: Arc<dyn Fn() -> u64 + Sync + Send>,
    base: ClockSourceBase,
    coeff: Coeff,
    last_record: RwLock<(Instant, u64), LocalIrqDisabled>,
}

impl ClockSource {
    pub fn new(
        freq: u64,
        max_delay_secs: u64,
        read_cycles: Arc<dyn Fn() -> u64 + Sync + Send>,
    ) -> Self {
        let base = ClockSourceBase::new(freq, max_delay_secs);
        debug_assert!(max_delay_secs < 600);
        let coeff = Coeff::new(NANOS_PER_SECOND as u64, freq, max_delay_secs * freq);
        Self {
            read_cycles,
            base,
            coeff,
            last_record: RwLock::new((Instant::zero(), 0)),
        }
    }

    fn calculate_instant(&self) -> (Instant, u64) {
        let (instant_cycles, last_instant, last_cycles) = {
            let last_record = self.last_record.read();
            let (last_instant, last_cycles) = *last_record;
            (self.read_cycles(), last_instant, last_cycles)
        };

        let delta_nanos = match instant_cycles.checked_sub(last_cycles) {
            Some(delta) => self.cycles_to_nanos_lossy(delta),
            None => {
                ostd::warn!(
                    "The clock source becomes not reliable since TSC \
                    has moved backwards from {} to {}",
                    last_cycles,
                    instant_cycles
                );
                0
            }
        };
        let duration = Duration::from_nanos(delta_nanos);
        (last_instant + duration, instant_cycles)
    }

    fn cycles_to_nanos_lossy(&self, cycles: u64) -> u64 {
        let max_cycles = self.base.max_delay_secs * self.base.freq;
        if cycles <= max_cycles {
            self.coeff * cycles
        } else {
            ostd::warn!(
                "The clock source becomes not reliable since an \
                interval of {} cycles exceeds the maximum delay {}(s)",
                cycles,
                max_cycles
            );
            self.coeff * max_cycles
        }
    }

    fn update_last_record(&self, record: (Instant, u64)) {
        *self.last_record.write() = record;
    }

    pub fn read_cycles(&self) -> u64 {
        (self.read_cycles)()
    }

    pub fn last_record(&self) -> (Instant, u64) {
        return *self.last_record.read();
    }

    pub fn max_delay_secs(&self) -> u64 {
        self.base.max_delay_secs
    }

    pub fn coeff(&self) -> &Coeff {
        &self.coeff
    }

    pub fn freq(&self) -> u64 {
        self.base.freq
    }

    pub(crate) fn calibrate(&self, instant_cycles: u64) {
        self.update_last_record((Instant::zero(), instant_cycles));
    }

    pub(crate) fn update(&self) {
        let (instant, instant_cycles) = self.calculate_instant();
        self.update_last_record((instant, instant_cycles));
    }

    pub(crate) fn read_instant(&self) -> Instant {
        self.calculate_instant().0
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Instant {
    secs: u64,
    nanos: u32,
}

impl Instant {
    pub const fn zero() -> Self {
        Self { secs: 0, nanos: 0 }
    }

    pub fn new(secs: u64, nanos: u32) -> Self {
        Self { secs, nanos }
    }

    pub fn secs(&self) -> u64 {
        self.secs
    }

    pub fn nanos(&self) -> u32 {
        self.nanos
    }
}

impl From<Duration> for Instant {
    fn from(value: Duration) -> Self {
        Self {
            secs: value.as_secs(),
            nanos: value.subsec_nanos(),
        }
    }
}

impl Add<Duration> for Instant {
    type Output = Instant;

    fn add(self, other: Duration) -> Self::Output {
        let mut secs = self.secs + other.as_secs();
        let mut nanos = self.nanos + other.subsec_nanos();
        if nanos >= NANOS_PER_SECOND {
            secs += 1;
            nanos -= NANOS_PER_SECOND;
        }
        Instant::new(secs, nanos)
    }
}

#[derive(Clone, Copy, Debug)]
struct ClockSourceBase {
    freq: u64,
    max_delay_secs: u64,
}

impl ClockSourceBase {
    fn new(freq: u64, max_delay_secs: u64) -> Self {
        let max_delay_secs = max(2, max_delay_secs);
        ClockSourceBase {
            freq,
            max_delay_secs,
        }
    }
}
