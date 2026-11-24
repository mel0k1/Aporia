pub(super) fn write_cr2_raw(value: u64) {
    unsafe { core::arch::asm!("mov cr2, {}", in(reg) value) };
}

pub(super) fn read_cr3_raw() -> usize {
    let value: usize;

    unsafe { core::arch::asm!("mov {}, cr3", out(reg) value) };
    value
}
