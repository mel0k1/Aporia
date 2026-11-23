pub use core::ops::{BitAnd as And, BitOr as Or, Not};
use core::unimplemented;

pub trait Bool {}

pub struct True;

pub struct False;

impl Bool for True {}
impl Bool for False {}

impl Not for True {
    type Output = False;

    fn not(self) -> Self::Output {
        unimplemented!("not supposed to be used")
    }
}

impl Not for False {
    type Output = True;

    fn not(self) -> Self::Output {
        unimplemented!("not supposed to be used")
    }
}

impl<B: Bool> And<B> for True {
    type Output = B;

    fn bitand(self, _rhs: B) -> Self::Output {
        unimplemented!("not supposed to be used")
    }
}

impl<B: Bool> And<B> for False {
    type Output = False;

    fn bitand(self, _rhs: B) -> Self::Output {
        unimplemented!("not supposed to be used")
    }
}

impl<B: Bool> Or<B> for True {
    type Output = True;

    fn bitor(self, _rhs: B) -> Self::Output {
        unimplemented!("not supposed to be used")
    }
}

impl<B: Bool> Or<B> for False {
    type Output = B;

    fn bitor(self, _rhs: B) -> Self::Output {
        unimplemented!("not supposed to be used")
    }
}

pub type NotOp<B> = <B as Not>::Output;
pub type AndOp<B0, B1> = <B0 as And<B1>>::Output;
pub type OrOp<B0, B1> = <B0 as Or<B1>>::Output;

pub trait IsTrue {}
pub trait IsFalse {}

impl IsTrue for True {}
impl IsFalse for False {}
