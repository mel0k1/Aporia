#![no_std]

pub mod batch;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IoError {
    Unsupported,
    OutOfSpace,
    Failed,
}
