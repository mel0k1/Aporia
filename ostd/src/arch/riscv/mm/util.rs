use crate::arch::trap::SSTATUS_SUM;

core::arch::global_asm!(include_str!("memcpy_fallible.S"), SSTATUS_SUM = const SSTATUS_SUM);
core::arch::global_asm!(include_str!("memset_fallible.S"), SSTATUS_SUM = const SSTATUS_SUM);

core::arch::global_asm!(include_str!("atomic_load_fallible.S"), SSTATUS_SUM = const SSTATUS_SUM);
core::arch::global_asm!(include_str!("atomic_cmpxchg_fallible.S"), SSTATUS_SUM = const SSTATUS_SUM);

unsafe extern "C" {
    pub(crate) fn __memcpy_fallible(dst: *mut u8, src: *const u8, size: usize) -> usize;
    pub(crate) fn __memset_fallible(dst: *mut u8, value: u8, size: usize) -> usize;

    pub(crate) fn __atomic_load_fallible(ptr: *const u32) -> u64;
    pub(crate) fn __atomic_cmpxchg_fallible(ptr: *mut u32, old_val: u32, new_val: u32) -> u64;
}
