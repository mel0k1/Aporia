use core::ops::Deref;

use crate::prelude::Result;

pub trait RandomInit: Default {
    fn random() -> Self;
}

pub trait Aead {
    type Key: Deref<Target = [u8]> + RandomInit;
    type Iv: Deref<Target = [u8]> + RandomInit;
    type Mac: Deref<Target = [u8]> + RandomInit;

    fn encrypt(
        &self,
        input: &[u8],
        key: &Self::Key,
        iv: &Self::Iv,
        aad: &[u8],
        output: &mut [u8],
    ) -> Result<Self::Mac>;

    fn decrypt(
        &self,
        input: &[u8],
        key: &Self::Key,
        iv: &Self::Iv,
        aad: &[u8],
        mac: &Self::Mac,
        output: &mut [u8],
    ) -> Result<()>;
}

pub(crate) trait Skcipher {
    type Key: Deref<Target = [u8]> + RandomInit;
    type Iv: Deref<Target = [u8]> + RandomInit;

    fn encrypt(
        &self,
        input: &[u8],
        key: &Self::Key,
        iv: &Self::Iv,
        output: &mut [u8],
    ) -> Result<()>;

    fn decrypt(
        &self,
        input: &[u8],
        key: &Self::Key,
        iv: &Self::Iv,
        output: &mut [u8],
    ) -> Result<()>;
}

pub trait Rng {
    fn new(seed: &[u8]) -> Self;

    fn fill_bytes(&self, dest: &mut [u8]) -> Result<()>;
}
