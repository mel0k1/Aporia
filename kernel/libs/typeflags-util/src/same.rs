use crate::bool::{Bool, False, True};

pub trait SameAs<T> {
    type Output: Bool;
}

impl<T> SameAs<T> for T {
    type Output = True;
}

impl SameAs<False> for True {
    type Output = False;
}

impl SameAs<True> for False {
    type Output = False;
}

pub type SameAsOp<T, U> = <T as SameAs<U>>::Output;
