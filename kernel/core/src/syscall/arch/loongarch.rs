//! System call dispatch in the LoongArch architecture.

#[path = "./generic.rs"]
mod generic;

generic::define_syscalls_with_generic_syscall_table! {}
