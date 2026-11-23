use core::time::Duration;

use ostd::arch::{
    kernel::{PvclockTimeSnapshot, read_pvclock_time_snapshot, read_wall_clock_epoch},
    read_tsc,
};

use super::Driver;
use crate::SystemTime;

pub(super) struct RtcKvmClock {
    clock: KvmWallClock,
}

impl Driver for RtcKvmClock {
    fn try_new() -> Option<Self> {
        let clock = KvmWallClock::setup()?;
        Some(Self { clock })
    }

    fn read_rtc(&self) -> SystemTime {
        let duration = self.clock.read_wall_clock();
        SystemTime::new(duration)
    }
}

struct KvmWallClock {
    epoch: Duration,
    snapshot: PvclockTimeSnapshot,
}

impl KvmWallClock {
    fn setup() -> Option<Self> {
        let epoch = read_wall_clock_epoch()?;
        let snapshot = read_pvclock_time_snapshot()?;

        if (snapshot.tsc_shift as i32).unsigned_abs() >= 64 {
            return None;
        }

        Some(Self { epoch, snapshot })
    }

    fn read_wall_clock(&self) -> Duration {
        let clock_ns = snapshot_clock_ns(&self.snapshot, read_tsc());

        self.epoch + Duration::from_nanos(clock_ns)
    }
}

fn snapshot_clock_ns(snapshot: &PvclockTimeSnapshot, tsc: u64) -> u64 {
    let delta = tsc.wrapping_sub(snapshot.tsc_timestamp);
    snapshot
        .system_time
        .saturating_add(scale_delta(snapshot, delta))
}

fn scale_delta(snapshot: &PvclockTimeSnapshot, delta: u64) -> u64 {
    let val = u128::from(delta) * u128::from(snapshot.tsc_to_system_mul);

    let res = if snapshot.tsc_shift >= 32 {
        val << ((snapshot.tsc_shift - 32) as u32)
    } else {
        val >> ((32 - snapshot.tsc_shift) as u32)
    };

    u64::try_from(res).unwrap_or(u64::MAX)
}
