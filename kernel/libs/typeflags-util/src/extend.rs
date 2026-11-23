use crate::{Cons, Nil};

pub trait SetExtend<T> {
    type Output;
}

pub type SetExtendOp<Set, T> = <Set as SetExtend<T>>::Output;

impl<T> SetExtend<T> for Nil {
    type Output = Cons<T, Nil>;
}
