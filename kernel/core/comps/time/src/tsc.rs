use alloc::sync::Arc;
use core::sync::atomic::{AtomicU64, Ordering};

use ostd::{
    arch::{read_tsc, tsc_freq},
    timer::{self, TIMER_FREQ},
};
use spin::Once;

use crate::{
    START_TIME, VDSO_DATA_HIGH_RES_UPDATE_FN,
    clocksource::{ClockSource, Instant},
};

pub(super) static CLOCK: Once<Arc<ClockSource>> = Once::new();

const MAX_DELAY_SECS: u64 = 100;

pub(super) fn init() {
    init_clock();
    calibrate();
    init_timer();
}

fn init_clock() {
    CLOCK.call_once(|| {
        Arc::new(ClockSource::new(
            tsc_freq(),
            MAX_DELAY_SECS,
            Arc::new(read_tsc),
        ))
    });
}

fn calibrate() {
    let clock = CLOCK.get().unwrap();
    let cycles = clock.read_cycles();
    clock.calibrate(cycles);
    START_TIME.call_once(|| crate::RTC_DRIVER.get().unwrap().read_rtc());
}

pub(super) fn read_instant() -> Instant {
    let clock = CLOCK.get().unwrap();
    clock.read_instant()
}

fn update_clocksource() {
    let clock = CLOCK.get().unwrap();
    clock.update();

    if let Some(update_fn) = VDSO_DATA_HIGH_RES_UPDATE_FN.get() {
        let (last_instant, last_cycles) = clock.last_record();
        update_fn(last_instant, last_cycles);
    }
}

static TSC_UPDATE_COUNTER: AtomicU64 = AtomicU64::new(1);

fn init_timer() {
    const VDSO_UPDATE_FREQ: u64 = 10;
    let delay_counts = TIMER_FREQ / VDSO_UPDATE_FREQ;

    let update = move || {
        let counter = TSC_UPDATE_COUNTER.fetch_add(1, Ordering::Relaxed);

        if counter.is_multiple_of(delay_counts) {
            update_clocksource();
        }
    };

    timer::register_callback_on_cpu(update);
}
