#![no_std]
pub mod assert;
pub mod bool;
pub mod extend;
pub mod if_;
pub mod same;
pub mod set;

pub use assert::AssertTypeSame;

pub use crate::{
    bool::{And, AndOp, False, IsFalse, IsTrue, Not, NotOp, Or, OrOp, True},
    extend::{SetExtend, SetExtendOp},
    same::{SameAs, SameAsOp},
    set::{Cons, Nil, Set, SetContain, SetContainOp, SetInclude, SetIncludeOp},
};
