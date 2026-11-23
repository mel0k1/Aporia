use core::{
    sync::atomic::{Ordering, fence},
    time::Duration,
};

use spin::Once;
use x86::msr;

use crate::{
    arch::{cpu::cpuid, io::io_mem::read_once},
    impl_frame_meta_for,
    mm::{self, Frame, FrameAllocOptions, HasPaddr},
};

const KVM_FEATURE_CLOCKSOURCE2: u32 = 3;
const MSR_KVM_SYSTEM_TIME_NEW: u32 = 0x4b56_4d01;
const MSR_KVM_WALL_CLOCK_NEW: u32 = 0x4b56_4d00;
const KVM_MSR_ENABLE_BIT: u64 = 1;

const MAX_RETRIES: usize = 1_000_000;

const NANOS_PER_SECOND: u32 = 1_000_000_000;

pub fn read_pvclock_time_snapshot() -> Option<PvclockTimeSnapshot> {
    PvclockPage::setup()?.read_time_snapshot()
}

pub fn read_wall_clock_epoch() -> Option<Duration> {
    WallClockPage::setup()?.read_epoch()
}

pub(super) fn determine_tsc_freq() -> Option<u64> {
    let pvclock_page = PvclockPage::setup()?;
    let time_snapshot = pvclock_page.read_time_snapshot()?;
    time_snapshot.tsc_freq_hz()
}

fn has_kvm_clocksource2() -> bool {
    cpuid::query_if_running_under_kvm() && cpuid::query_kvm_feature(KVM_FEATURE_CLOCKSOURCE2)
}

trait PvclockVersion {
    fn read_version(&self) -> u32;

    fn read_begin(&self) -> u32 {
        let version = self.read_version() & !1;

        fence(Ordering::Acquire);
        version
    }

    fn is_version_changed(&self, version: u32) -> bool {
        fence(Ordering::Acquire);
        self.read_version() != version
    }
}

struct PvclockPage(Frame<PvclockPageMeta>);

impl PvclockPage {
    fn setup() -> Option<&'static Self> {
        static PVCLOCK_PAGE: Once<PvclockPage> = Once::new();

        if !has_kvm_clocksource2() {
            return None;
        }

        if let Some(page) = PVCLOCK_PAGE.get() {
            return Some(page);
        }

        let frame = FrameAllocOptions::new()
            .alloc_frame_with(PvclockPageMeta)
            .ok()?;
        Some(PVCLOCK_PAGE.call_once(|| {
            unsafe {
                msr::wrmsr(
                    MSR_KVM_SYSTEM_TIME_NEW,
                    frame.paddr() as u64 | KVM_MSR_ENABLE_BIT,
                )
            };
            Self(frame)
        }))
    }

    fn read_time_snapshot(&self) -> Option<PvclockTimeSnapshot> {
        for _ in 0..MAX_RETRIES {
            let version = self.read_begin();

            let tsc_timestamp = self.read_tsc_timestamp();
            let system_time = self.read_system_time();
            let tsc_to_system_mul = self.read_tsc_to_system_mul();
            let tsc_shift = self.read_tsc_shift();

            if !self.is_version_changed(version) && tsc_to_system_mul != 0 {
                return Some(PvclockTimeSnapshot {
                    tsc_timestamp,
                    system_time,
                    tsc_to_system_mul,
                    tsc_shift,
                });
            }

            core::hint::spin_loop();
        }

        None
    }

    fn read_tsc_timestamp(&self) -> u64 {
        unsafe { read_once(core::ptr::addr_of!((*self.as_ptr()).tsc_timestamp)) }
    }

    fn read_system_time(&self) -> u64 {
        unsafe { read_once(core::ptr::addr_of!((*self.as_ptr()).system_time)) }
    }

    fn read_tsc_to_system_mul(&self) -> u32 {
        unsafe { read_once(core::ptr::addr_of!((*self.as_ptr()).tsc_to_system_mul)) }
    }

    fn read_tsc_shift(&self) -> i8 {
        unsafe { read_once(core::ptr::addr_of!((*self.as_ptr()).tsc_shift)) }
    }

    fn as_ptr(&self) -> *const PvclockVcpuTimeInfo {
        mm::paddr_to_vaddr(self.0.paddr()) as *const PvclockVcpuTimeInfo
    }
}

impl PvclockVersion for PvclockPage {
    fn read_version(&self) -> u32 {
        unsafe { read_once(core::ptr::addr_of!((*self.as_ptr()).version)) }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PvclockTimeSnapshot {
    pub tsc_timestamp: u64,
    pub system_time: u64,
    pub tsc_to_system_mul: u32,
    pub tsc_shift: i8,
}

impl PvclockTimeSnapshot {
    fn tsc_freq_hz(&self) -> Option<u64> {
        let base_khz = (1_000_000u64 << 32).checked_div(u64::from(self.tsc_to_system_mul))?;

        let tsc_khz = if self.tsc_shift < 0 {
            base_khz.checked_shl((self.tsc_shift as i32).unsigned_abs())?
        } else {
            base_khz.checked_shr(self.tsc_shift as u32)?
        };

        let freq = tsc_khz.checked_mul(1000)?;
        (freq != 0).then_some(freq)
    }
}

struct WallClockPage(Frame<WallClockPageMeta>);

impl WallClockPage {
    fn setup() -> Option<&'static Self> {
        static WALL_CLOCK_PAGE: Once<WallClockPage> = Once::new();

        if !has_kvm_clocksource2() {
            return None;
        }

        if let Some(page) = WALL_CLOCK_PAGE.get() {
            return Some(page);
        }

        let frame = FrameAllocOptions::new()
            .alloc_frame_with(WallClockPageMeta)
            .ok()?;
        Some(WALL_CLOCK_PAGE.call_once(|| {
            unsafe { msr::wrmsr(MSR_KVM_WALL_CLOCK_NEW, frame.paddr() as u64) };
            Self(frame)
        }))
    }

    fn read_epoch(&self) -> Option<Duration> {
        for _ in 0..MAX_RETRIES {
            let version = self.read_begin();

            let sec = self.read_sec();
            let nsec = self.read_nsec();

            if !self.is_version_changed(version) {
                if nsec >= NANOS_PER_SECOND {
                    return None;
                }
                return Some(Duration::new(u64::from(sec), nsec));
            }

            core::hint::spin_loop();
        }

        None
    }

    fn read_sec(&self) -> u32 {
        unsafe { read_once(core::ptr::addr_of!((*self.as_ptr()).sec)) }
    }

    fn read_nsec(&self) -> u32 {
        unsafe { read_once(core::ptr::addr_of!((*self.as_ptr()).nsec)) }
    }

    fn as_ptr(&self) -> *const PvclockWallClock {
        mm::paddr_to_vaddr(self.0.paddr()) as *const PvclockWallClock
    }
}

impl PvclockVersion for WallClockPage {
    fn read_version(&self) -> u32 {
        unsafe { read_once(core::ptr::addr_of!((*self.as_ptr()).version)) }
    }
}

#[repr(C, packed)]
struct PvclockVcpuTimeInfo {
    version: u32,
    _pad0: u32,
    tsc_timestamp: u64,
    system_time: u64,
    tsc_to_system_mul: u32,
    tsc_shift: i8,
    _flags: u8,
    _pad: [u8; 2],
}

#[repr(C, packed)]
struct PvclockWallClock {
    version: u32,
    sec: u32,
    nsec: u32,
}

struct PvclockPageMeta;
impl_frame_meta_for!(PvclockPageMeta);

struct WallClockPageMeta;
impl_frame_meta_for!(WallClockPageMeta);
