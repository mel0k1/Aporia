//! Pipes implementation.
//!
//! This module provides both anonymous and named pipes for inter-process communication.

pub(crate) use anon_pipe::{AnonPipeInode, new_file_pair};
pub(crate) use common::Pipe;

mod anon_pipe;
mod common;
