use core::mem;

use spin::Once;

fn tsc_factors() -> (u64, u64) {
    static FACTORS: Once<(u64, u64)> = Once::new();
    *FACTORS.call_once(|| {
        let freq = ostd::arch::tsc_freq();
        assert_ne!(freq, 0);
        let mut a = 1_000_000_000;
        let mut b = freq;
        if a < b {
            mem::swap(&mut a, &mut b);
        }
        while a > 1 && b > 1 {
            let t = a;
            a = b;
            b = t % b;
        }

        let gcd = if a <= 1 { b } else { a };
        (1_000_000_000 / gcd, freq / gcd)
    })
}

pub(crate) const BASE_SLICE_NS: u64 = 750_000;

pub(crate) const MIN_PERIOD_NS: u64 = 6_000_000;

fn consts() -> (u64, u64) {
    static CONSTS: Once<(u64, u64)> = Once::new();
    *CONSTS.call_once(|| {
        let (a, b) = tsc_factors();
        (BASE_SLICE_NS * b / a, MIN_PERIOD_NS * b / a)
    })
}

pub(crate) fn base_slice_clocks() -> u64 {
    consts().0
}

pub(crate) fn min_period_clocks() -> u64 {
    consts().1
}
