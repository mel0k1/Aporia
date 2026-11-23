#![cfg_attr(not(test), no_std)]

pub trait AlignExt {
    fn align_up(self, power_of_two: Self) -> Self;

    fn align_down(self, power_of_two: Self) -> Self;
}

macro_rules! impl_align_ext {
    ($( $uint_type:ty ),+,) => {
        $(
            impl AlignExt for $uint_type {
                #[inline]
                fn align_up(self, align: Self) -> Self {
                    assert!(align.is_power_of_two() && align >= 2);
                    self.checked_add(align - 1).unwrap() & !(align - 1)
                }

                #[inline]
                fn align_down(self, align: Self) -> Self {
                    assert!(align.is_power_of_two() && align >= 2);
                    self & !(align - 1)
                }
            }
        )*
    }
}

impl_align_ext! {
    u8,
    u16,
    u32,
    u64,
    usize,
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn align_up() {
        let input_ns = [0usize, 1, 2, 9, 15, 21, 32, 47, 50];
        let input_as = [2usize, 2, 2, 2, 4, 4, 8, 8, 8];
        let output_ns = [0usize, 2, 2, 10, 16, 24, 32, 48, 56];

        for i in 0..input_ns.len() {
            let n = input_ns[i];
            let a = input_as[i];
            let n2 = output_ns[i];
            assert!(n.align_up(a) == n2);
        }
    }

    #[test]
    fn align_down() {
        let input_ns = [0usize, 1, 2, 9, 15, 21, 32, 47, 50];
        let input_as = [2usize, 2, 2, 2, 4, 4, 8, 8, 8];
        let output_ns = [0usize, 0, 2, 8, 12, 20, 32, 40, 48];

        for i in 0..input_ns.len() {
            let n = input_ns[i];
            let a = input_as[i];
            let n2 = output_ns[i];
            assert!(n.align_down(a) == n2);
        }
    }
}
