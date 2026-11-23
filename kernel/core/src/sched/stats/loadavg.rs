use core::sync::atomic::{AtomicU64, Ordering::Relaxed};

use aster_util::fixed_point::FixedU32;
use ostd::{
    sync::{LocalIrqDisabled, RwLock},
    timer::{self, TIMER_FREQ},
};

pub(crate) type LoadAvgFixed = FixedU32<11>;

const LOAD_FREQ: u64 = 5 * TIMER_FREQ + 1;
const EXP_1: LoadAvgFixed = LoadAvgFixed::from_raw(1884);
const EXP_5: LoadAvgFixed = LoadAvgFixed::from_raw(2014);
const EXP_15: LoadAvgFixed = LoadAvgFixed::from_raw(2037);

static LOAD_AVG: RwLock<[LoadAvgFixed; 3], LocalIrqDisabled> = RwLock::new([LoadAvgFixed::ZERO; 3]);

static LOAD_AVG_NEXT_UPDATE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn get_loadavg() -> [LoadAvgFixed; 3] {
    *LOAD_AVG.read()
}

pub(crate) fn update_loadavg<F>(get_load: F)
where
    F: Fn() -> u32,
{
    let jiffies = timer::Jiffies::elapsed().as_u64();

    if jiffies < LOAD_AVG_NEXT_UPDATE.load(Relaxed) {
        return;
    }

    LOAD_AVG_NEXT_UPDATE.store(jiffies + LOAD_FREQ, Relaxed);

    let new_load = LoadAvgFixed::saturating_from_num(get_load());

    let mut load = LOAD_AVG.write();

    load[0] = calc_loadavg(load[0], EXP_1, new_load);
    load[1] = calc_loadavg(load[1], EXP_5, new_load);
    load[2] = calc_loadavg(load[2], EXP_15, new_load);
}

fn calc_loadavg(old_load: LoadAvgFixed, exp: LoadAvgFixed, new_load: LoadAvgFixed) -> LoadAvgFixed {
    old_load * exp + new_load * (LoadAvgFixed::ONE - exp)
}
