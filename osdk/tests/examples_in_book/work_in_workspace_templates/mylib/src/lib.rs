pub fn available_memory() -> usize {
    let regions = &ostd::boot::boot_info().memory_regions;
    regions.iter().map(|region| region.len()).sum()
}
