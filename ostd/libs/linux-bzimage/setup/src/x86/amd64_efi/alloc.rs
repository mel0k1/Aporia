use core::mem::MaybeUninit;

use uefi::boot::AllocateType;

pub(crate) fn alloc_at(addr: usize, size: usize) -> &'static mut [MaybeUninit<u8>] {
    assert_ne!(addr, 0, "the address to allocate is zero");
    assert!(
        addr.checked_add(size).is_some(),
        "the range to allocate overflows"
    );

    let allocated = alloc_pages(AllocateType::Address(addr as u64), size);
    assert_eq!(
        allocated.as_ptr().addr(),
        addr,
        "the allocated address is not the request address"
    );

    allocated
}

pub(super) fn alloc_pages(ty: AllocateType, size: usize) -> &'static mut [MaybeUninit<u8>] {
    assert!(
        size <= isize::MAX as usize,
        "the size to allocate exceeds `isize::MAX`"
    );

    let allocated = uefi::boot::allocate_pages(
        ty,
        uefi::boot::MemoryType::LOADER_DATA,
        size.div_ceil(super::efi::PAGE_SIZE as usize),
    )
    .expect("the UEFI allocation fails");

    unsafe { core::slice::from_raw_parts_mut(allocated.as_ptr().cast(), size) }
}
