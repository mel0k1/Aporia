use alloc::sync::Arc;
use core::{mem::ManuallyDrop, time::Duration};

use aster_time::{Instant, read_monotonic_time};
use aster_util::coeff::Coeff;
use ostd::{
    const_assert,
    mm::{PAGE_SIZE, UFrame, VmIo, VmIoOnce, VmReader},
    sync::SpinLock,
};
use ostd_pod::IntoBytes;
use spin::Once;

use crate::{
    syscall::ClockId,
    time::{
        clocks::MonotonicClock,
        timer::{Timeout, TimerGuard},
    },
    vm::page_cache::{Vmo, VmoMapMode, VmoOptions},
};

const CLOCK_TAI: usize = 11;
const VDSO_BASES: usize = CLOCK_TAI + 1;
const DEFAULT_CLOCK_MODE: VdsoClockMode = VdsoClockMode::Tsc;

static VDSO: Once<Arc<Vdso>> = Once::new();

#[derive(Clone, Copy, Debug)]
enum VdsoClockMode {
    None = 0,
    Tsc = 1,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct VdsoInstant {
    secs: u64,
    nanos_info: u64,
}

impl VdsoInstant {
    const fn zero() -> Self {
        Self {
            secs: 0,
            nanos_info: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct ArchVdsoData {}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct VdsoData {
    seq: u32,

    clock_mode: i32,
    last_cycles: u64,
    mask: u64,
    mult: u32,
    shift: u32,
    basetime: [VdsoInstant; VDSO_BASES],

    tz_minuteswest: i32,
    tz_dsttime: i32,
    hrtimer_res: u32,
    __unused: u32,

    arch_data: ArchVdsoData,
}

const HIGH_RES_CLOCK_IDS: [ClockId; 4] = [
    ClockId::CLOCK_REALTIME,
    ClockId::CLOCK_MONOTONIC,
    ClockId::CLOCK_MONOTONIC_RAW,
    ClockId::CLOCK_BOOTTIME,
];

const COARSE_RES_CLOCK_IDS: [ClockId; 2] = [
    ClockId::CLOCK_REALTIME_COARSE,
    ClockId::CLOCK_MONOTONIC_COARSE,
];

impl VdsoData {
    const fn empty() -> Self {
        VdsoData {
            seq: 0,
            clock_mode: VdsoClockMode::None as i32,
            last_cycles: 0,
            mask: u64::MAX,
            mult: 0,
            shift: 0,
            basetime: [VdsoInstant::zero(); VDSO_BASES],
            tz_minuteswest: 0,
            tz_dsttime: 0,
            hrtimer_res: 0,
            __unused: 0,
            arch_data: ArchVdsoData {},
        }
    }

    fn init(&mut self) {
        let clocksource = aster_time::default_clocksource();
        let coeff = clocksource.coeff();
        self.set_clock_mode(DEFAULT_CLOCK_MODE);
        self.set_coeff(coeff);

        let (last_instant, last_cycles) = clocksource.last_record();
        self.update_high_res_instant(last_instant, last_cycles);
        self.update_coarse_res_instant(last_instant);
    }

    fn set_clock_mode(&mut self, mode: VdsoClockMode) {
        self.clock_mode = mode as i32;
    }

    fn set_coeff(&mut self, coeff: &Coeff) {
        self.mult = coeff.mult();
        self.shift = coeff.shift();
    }

    fn update_clock_instant(&mut self, clockid: usize, secs: u64, nanos_info: u64) {
        self.basetime[clockid].secs = secs;
        self.basetime[clockid].nanos_info = nanos_info;
    }

    fn update_high_res_instant(&mut self, instant: Instant, instant_cycles: u64) {
        self.last_cycles = instant_cycles;
        for clock_id in HIGH_RES_CLOCK_IDS {
            let (secs, nanos) = if clock_id == ClockId::CLOCK_REALTIME {
                let base = Self::realtime_duration(instant);
                (base.as_secs(), base.subsec_nanos() as u64)
            } else {
                (instant.secs(), instant.nanos() as u64)
            };

            self.update_clock_instant(clock_id as usize, secs, nanos << (self.shift as u64));
        }
    }

    fn update_coarse_res_instant(&mut self, instant: Instant) {
        for clock_id in COARSE_RES_CLOCK_IDS {
            let (secs, nanos) = if clock_id == ClockId::CLOCK_REALTIME_COARSE {
                let base = Self::realtime_duration(instant);
                (base.as_secs(), base.subsec_nanos() as u64)
            } else {
                (instant.secs(), instant.nanos() as u64)
            };
            self.update_clock_instant(clock_id as usize, secs, nanos);
        }
    }

    fn realtime_duration(instant: Instant) -> Duration {
        #[cfg(not(ktest))]
        let start_offset = aster_time::read_start_time().since_epoch();
        #[cfg(ktest)]
        let start_offset = test::TEST_START_OFFSET;

        start_offset + Duration::new(instant.secs(), instant.nanos())
    }
}

macro_rules! vdso_data_field_offset {
    ($field:ident) => {
        VDSO_VMO_LAYOUT.data_offset + core::mem::offset_of!(VdsoData, $field)
    };
}

struct Vdso {
    data: SpinLock<VdsoData>,
    vmo: Arc<Vmo>,
    data_frame: UFrame,
}

#[cfg(target_arch = "x86_64")]
const PREBUILT_VDSO_LIB: &[u8] =
    include_bytes!(concat!(env!("VDSO_LIBRARY_DIR"), "/vdso_x86_64.so"));
#[cfg(target_arch = "riscv64")]
const PREBUILT_VDSO_LIB: &[u8] =
    include_bytes!(concat!(env!("VDSO_LIBRARY_DIR"), "/vdso_riscv64.so"));
#[cfg(target_arch = "aarch64")]
const PREBUILT_VDSO_LIB: &[u8] =
    include_bytes!(concat!(env!("VDSO_LIBRARY_DIR"), "/vdso_aarch64.so"));

#[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
pub(crate) const __VDSO_RT_SIGRETURN_OFFSET: usize = cfg_select! {
    target_arch = "riscv64" => 0x5b0,
    target_arch = "aarch64" => 0x538,
};

impl Vdso {
    fn new() -> Self {
        let mut vdso_data = VdsoData::empty();
        vdso_data.init();

        let (vdso_vmo, data_frame) = {
            let vmo_options = VmoOptions::new(VDSO_VMO_LAYOUT.size);
            let vdso_vmo = vmo_options.alloc().unwrap();
            let mut reader = VmReader::from(vdso_data.as_bytes()).to_fallible();
            vdso_vmo
                .write(VDSO_VMO_LAYOUT.data_offset, &mut reader)
                .unwrap();

            let mut reader = VmReader::from(
                &PREBUILT_VDSO_LIB[..VDSO_VMO_LAYOUT
                    .text_segment_size
                    .min(PREBUILT_VDSO_LIB.len())],
            )
            .to_fallible();
            vdso_vmo
                .write(VDSO_VMO_LAYOUT.text_segment_offset, &mut reader)
                .unwrap();

            let (data_frame, _) = vdso_vmo
                .try_commit_page(0, VmoMapMode::SharedWrite)
                .unwrap();
            (vdso_vmo, data_frame)
        };

        Self {
            data: SpinLock::new(vdso_data),
            vmo: vdso_vmo,
            data_frame: data_frame.into(),
        }
    }

    fn update_high_res_instant(&self, instant: Instant, instant_cycles: u64) {
        let mut data = self.data.lock();

        data.update_high_res_instant(instant, instant_cycles);

        self.begin_update_data_frame(&mut data);

        self.data_frame
            .write_val(vdso_data_field_offset!(last_cycles), &instant_cycles)
            .unwrap();
        for clock_id in HIGH_RES_CLOCK_IDS {
            self.update_data_frame_instant(clock_id, &mut data);
        }

        self.finish_update_data_frame(&mut data);
    }

    fn update_coarse_res_instant(&self, instant: Instant) {
        let mut data = self.data.lock();

        data.update_coarse_res_instant(instant);

        self.begin_update_data_frame(&mut data);

        for clock_id in COARSE_RES_CLOCK_IDS {
            self.update_data_frame_instant(clock_id, &mut data);
        }

        self.finish_update_data_frame(&mut data);
    }

    fn begin_update_data_frame(&self, data: &mut VdsoData) {
        debug_assert!(data.seq.is_multiple_of(2));

        data.seq = data.seq.wrapping_add(1);
        self.data_frame
            .write_once(vdso_data_field_offset!(seq), &data.seq)
            .unwrap();
    }

    fn update_data_frame_instant(&self, clockid: ClockId, data: &mut VdsoData) {
        let clock_index = clockid as usize;

        let secs_offset =
            vdso_data_field_offset!(basetime) + clock_index * size_of::<VdsoInstant>();
        let nanos_info_offset = vdso_data_field_offset!(basetime)
            + core::mem::offset_of!(VdsoInstant, nanos_info)
            + clock_index * size_of::<VdsoInstant>();
        self.data_frame
            .write_val(secs_offset, &data.basetime[clock_index].secs)
            .unwrap();
        self.data_frame
            .write_val(nanos_info_offset, &data.basetime[clock_index].nanos_info)
            .unwrap();
    }

    fn finish_update_data_frame(&self, data: &mut VdsoData) {
        debug_assert!(!data.seq.is_multiple_of(2));

        data.seq = data.seq.wrapping_add(1);
        self.data_frame
            .write_once(vdso_data_field_offset!(seq), &data.seq)
            .unwrap();
    }
}

fn update_vdso_high_res_instant(instant: Instant, instant_cycles: u64) {
    VDSO.get()
        .unwrap()
        .update_high_res_instant(instant, instant_cycles);
}

fn update_vdso_coarse_res_instant(_guard: TimerGuard) {
    let instant = Instant::from(read_monotonic_time());
    VDSO.get().unwrap().update_coarse_res_instant(instant);
}

fn init_vdso() {
    let vdso = Vdso::new();
    VDSO.call_once(|| Arc::new(vdso));
}

pub(super) fn init_in_first_kthread() {
    init_vdso();

    aster_time::VDSO_DATA_HIGH_RES_UPDATE_FN.call_once(|| update_vdso_high_res_instant);

    let coarse_instant_timer = ManuallyDrop::new(
        MonotonicClock::timer_manager().create_timer(update_vdso_coarse_res_instant),
    );
    let mut timer_guard = coarse_instant_timer.lock();
    timer_guard.set_interval(Duration::from_millis(100));
    timer_guard.set_timeout(Timeout::After(Duration::from_millis(100)));
}

pub(crate) fn vdso_vmo() -> Option<Arc<Vmo>> {
    VDSO.get().map(|vdso| vdso.vmo.clone())
}

#[cfg(target_arch = "x86_64")]
pub(crate) const VDSO_VMO_LAYOUT: VdsoVmoLayout = VdsoVmoLayout {
    data_segment_offset: 0,
    data_segment_size: PAGE_SIZE,
    text_segment_offset: 4 * PAGE_SIZE,
    text_segment_size: PAGE_SIZE,
    data_offset: 0x80,

    size: 5 * PAGE_SIZE,
};

#[cfg(any(target_arch = "riscv64", target_arch = "aarch64"))]
pub(crate) const VDSO_VMO_LAYOUT: VdsoVmoLayout = VdsoVmoLayout {
    data_segment_offset: 0,
    data_segment_size: PAGE_SIZE,
    text_segment_offset: 2 * PAGE_SIZE,
    text_segment_size: PAGE_SIZE,
    data_offset: 0,

    size: 3 * PAGE_SIZE,
};

pub(crate) struct VdsoVmoLayout {
    pub data_segment_offset: usize,
    pub data_segment_size: usize,
    pub text_segment_offset: usize,
    pub text_segment_size: usize,
    pub data_offset: usize,
    pub size: usize,
}

const_assert!(
    VDSO_VMO_LAYOUT
        .data_segment_offset
        .is_multiple_of(PAGE_SIZE)
);
const_assert!(VDSO_VMO_LAYOUT.data_segment_size.is_multiple_of(PAGE_SIZE));
const_assert!(
    VDSO_VMO_LAYOUT
        .text_segment_offset
        .is_multiple_of(PAGE_SIZE)
);
const_assert!(VDSO_VMO_LAYOUT.text_segment_size.is_multiple_of(PAGE_SIZE));
const_assert!(VDSO_VMO_LAYOUT.size.is_multiple_of(PAGE_SIZE));

const_assert!(
    VDSO_VMO_LAYOUT.data_offset + size_of::<VdsoData>()
        <= VDSO_VMO_LAYOUT.data_segment_offset + VDSO_VMO_LAYOUT.data_segment_size
);

#[cfg(ktest)]
mod test {
    use core::time::Duration;

    use aster_time::Instant;
    use aster_util::coeff::Coeff;
    use ostd::prelude::*;

    use super::VdsoData;
    use crate::syscall::ClockId;

    pub(super) const TEST_START_OFFSET: Duration = Duration::new(1_700_000_000, 800_000_000);

    #[ktest]
    fn realtime_duration_adds_fractional_seconds() {
        let base = VdsoData::realtime_duration(Instant::new(5, 100_000_000));
        assert_eq!(base, Duration::new(1_700_000_005, 900_000_000));

        let base = VdsoData::realtime_duration(Instant::new(5, 400_000_000));
        assert_eq!(base, Duration::new(1_700_000_006, 200_000_000));
    }

    #[ktest]
    fn realtime_clocks_store_fractional_seconds() {
        let instant = Instant::new(5, 400_000_000);
        let expected = TEST_START_OFFSET + Duration::new(instant.secs(), instant.nanos());

        let coeff = Coeff::new(1_000_000_000, 1, 1_u64 << 20);
        assert!(coeff.shift() > 0);

        let mut data = VdsoData::empty();
        data.set_coeff(&coeff);
        data.update_high_res_instant(instant, 0);
        data.update_coarse_res_instant(instant);

        let coarse = data.basetime[ClockId::CLOCK_REALTIME_COARSE as usize];
        let coarse_nanos = coarse.nanos_info;
        assert_eq!(coarse.secs, expected.as_secs());
        assert_eq!(coarse_nanos, expected.subsec_nanos() as u64);

        let realtime = data.basetime[ClockId::CLOCK_REALTIME as usize];
        let realtime_nanos = realtime.nanos_info >> coeff.shift();
        assert_eq!(realtime.secs, expected.as_secs());
        assert_eq!(realtime_nanos, expected.subsec_nanos() as u64);
    }
}
