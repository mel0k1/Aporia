//! System call dispatch in the ARM architecture.

#[path = "./generic.rs"]
mod generic;

generic::define_syscalls_with_generic_syscall_table! {}
