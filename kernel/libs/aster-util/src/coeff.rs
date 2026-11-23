use core::ops::Mul;

#[derive(Clone, Copy, Debug)]
pub struct Coeff {
    mult: u32,
    shift: u32,
    max_multiplier: u64,
}

impl Coeff {
    pub fn new(numerator: u64, denominator: u64, max_multiplier: u64) -> Self {
        let mut shift_acc: u32 = 32;
        debug_assert!(max_multiplier < (1 << 40));
        let mut tmp = max_multiplier >> 32;
        while tmp > 0 {
            tmp >>= 1;
            shift_acc -= 1;
        }

        let mut shift = 32;
        let mut mult = 0;
        while shift > 0 {
            mult = numerator << shift;
            mult += denominator / 2;
            mult /= denominator;
            if (mult >> shift_acc) == 0 {
                break;
            }
            shift -= 1;
        }
        Self {
            mult: mult as u32,
            shift,
            max_multiplier,
        }
    }

    pub fn mult(&self) -> u32 {
        self.mult
    }

    pub fn shift(&self) -> u32 {
        self.shift
    }
}

impl Mul<u64> for Coeff {
    type Output = u64;
    fn mul(self, rhs: u64) -> Self::Output {
        debug_assert!(rhs <= self.max_multiplier);
        (rhs * self.mult as u64) >> self.shift
    }
}

impl Mul<u32> for Coeff {
    type Output = u32;
    fn mul(self, rhs: u32) -> Self::Output {
        debug_assert!(rhs as u64 <= self.max_multiplier);
        ((rhs as u64 * self.mult as u64) >> self.shift) as u32
    }
}

#[cfg(ktest)]
mod test {
    use ostd::prelude::*;

    use super::*;

    #[ktest]
    fn calculation() {
        let coeff = Coeff::new(23456, 56789, 1_000_000_000);
        assert!(coeff * 0_u64 == 0);
        assert!(coeff * 100_u64 == 100 * 23456 / 56789);
        assert!(coeff * 1_000_000_000_u64 == 1_000_000_000 * 23456 / 56789);
    }
}
