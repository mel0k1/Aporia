mod bitmap;
mod crypto;
mod lazy_delete;

pub use self::crypto::{Aead, RandomInit, Rng};
pub(crate) use self::{bitmap::BitMap, crypto::Skcipher, lazy_delete::LazyDelete};

pub(crate) const fn align_up(x: usize, align: usize) -> usize {
    x.div_ceil(align) * align
}

pub(crate) const fn align_down(x: usize, align: usize) -> usize {
    (x / align) * align
}
