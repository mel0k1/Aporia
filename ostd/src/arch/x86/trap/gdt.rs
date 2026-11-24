//! Configure the Global Descriptor Table (GDT).

use core::{
    cell::UnsafeCell,
    mem::offset_of,
    sync::atomic::{AtomicU64, Ordering},
};

use x86_64::{
    PrivilegeLevel, VirtAddr,
    instructions::tables::{lgdt, load_tss},
    registers::segmentation::{CS, Segment},
    structures::{
        DescriptorTablePointer,
        gdt::{Descriptor, SegmentSelector},
        tss::TaskStateSegment,
    },
};

use crate::{
    const_assert,
    cpu::{
        CpuId,
        local::{CpuLocal, StaticCpuLocal},
    },
    cpu_local,
    irq::DisabledLocalIrqGuard,
    mm::Vaddr,
};

/// Initializes and loads the GDT and TSS.
///
/// The caller should only call this method once in the boot context for each available processor.
/// This is not a safety requirement, however, because calling this method again will do nothing
/// more than load the GDT and TSS with the same contents.
///
/// # Safety
///
/// The caller must ensure that no preemption can occur during the method, otherwise we may
/// accidentally load a wrong GDT and TSS that actually belongs to another CPU.
pub(super) unsafe fn init_on_cpu() {
    let gdt = GDT.get_on_cpu(CpuId::current_racy());

    let tss_ptr = UnsafeCell::raw_get(LOCAL_TSS.as_ptr());

    let tss_desc = unsafe { Descriptor::tss_segment_unchecked(tss_ptr) };
    let (tss0, tss1) = match tss_desc {
        Descriptor::SystemSegment(tss0, tss1) => (tss0, tss1),
        _ => unreachable!(),
    };
    gdt.tss0.store(tss0, Ordering::Relaxed);
    gdt.tss1.store(tss1, Ordering::Relaxed);

    assert_eq!(CS::get_reg(), KERNEL_CS);

    let gdtr = DescriptorTablePointer {
        limit: (size_of_val(gdt) - 1) as u16,
        base: VirtAddr::new(core::ptr::from_ref(gdt).addr() as u64),
    };

    unsafe { lgdt(&gdtr) };

    unsafe { load_tss(TSS_SEL) };
}

/// Returns the current CPU's GDT base address.
pub(in crate::arch) fn gdt_base(_guard: &DisabledLocalIrqGuard) -> Vaddr {
    GDT.as_ptr() as Vaddr
}

/// Returns the GDT limit.
pub(in crate::arch) fn gdt_limit() -> u16 {
    (size_of::<Gdt>() - 1) as u16
}

/// Returns the current CPU's TSS base address.
pub(in crate::arch) fn tss_base(_guard: &DisabledLocalIrqGuard) -> Vaddr {
    UnsafeCell::raw_get(LOCAL_TSS.as_ptr()) as Vaddr
}

#[unsafe(link_section = ".cpu_local_tss")]
static LOCAL_TSS: StaticCpuLocal<UnsafeCell<TaskStateSegment>> = {
    let tss = UnsafeCell::new(TaskStateSegment::new());

    unsafe { CpuLocal::__new_static(tss) }
};

#[repr(C)]
struct Gdt {
    _reserved: u64,
    kcode64: u64,
    kdata: u64,
    _kcode32: u64,
    _ucode32: u64,
    udata: u64,
    ucode64: u64,

    tss0: AtomicU64,
    tss1: AtomicU64,
}

cpu_local! {
    static GDT: Gdt = Gdt {
        _reserved: 0,
        kcode64: KCODE64,
        kdata: KDATA,
        _kcode32: 0,
        _ucode32: 0,
        udata: UDATA,
        ucode64: UCODE64,
        tss0: AtomicU64::new(0),
        tss1: AtomicU64::new(0),
    };
}

pub(in crate::arch) const KCODE64: u64 = 0x00AF_9B00_0000_FFFF;
pub(in crate::arch) const KDATA: u64 = 0x00CF_9300_0000_FFFF;

pub(in crate::arch) const KCODE32: u64 = 0x00CF_9B00_0000_FFFF;

const UCODE64: u64 = 0x00AF_FB00_0000_FFFF;
const UDATA: u64 = 0x00CF_F300_0000_FFFF;

pub(in crate::arch) const KERNEL_CS: SegmentSelector =
    SegmentSelector::new(1, PrivilegeLevel::Ring0);
const KERNEL_SS: SegmentSelector = SegmentSelector::new(2, PrivilegeLevel::Ring0);
const_assert!(KERNEL_CS.0 & OFFSET_MASK == offset_of!(Gdt, kcode64) as u16);
const_assert!(KERNEL_SS.0 & OFFSET_MASK == offset_of!(Gdt, kdata) as u16);

pub(super) const USER_CS: SegmentSelector = SegmentSelector::new(6, PrivilegeLevel::Ring3);
pub(super) const USER_SS: SegmentSelector = SegmentSelector::new(5, PrivilegeLevel::Ring3);
const_assert!(USER_CS.0 & OFFSET_MASK == offset_of!(Gdt, ucode64) as u16);
const_assert!(USER_SS.0 & OFFSET_MASK == offset_of!(Gdt, udata) as u16);

pub(in crate::arch) const TSS_SEL: SegmentSelector = SegmentSelector::new(7, PrivilegeLevel::Ring0);
const_assert!(TSS_SEL.0 & OFFSET_MASK == offset_of!(Gdt, tss0) as u16);
const_assert!(TSS_SEL.0 & OFFSET_MASK == (offset_of!(Gdt, tss1) - size_of::<u64>()) as u16);

pub(in crate::arch) const SYSRET_SEL: SegmentSelector =
    SegmentSelector::new(4, PrivilegeLevel::Ring3);
pub(in crate::arch) const SYSCALL_SEL: SegmentSelector =
    SegmentSelector::new(1, PrivilegeLevel::Ring0);
const_assert!(SYSRET_SEL.0 & OFFSET_MASK == (offset_of!(Gdt, udata) - size_of::<u64>()) as u16);
const_assert!(
    SYSRET_SEL.0 & OFFSET_MASK == (offset_of!(Gdt, ucode64) - size_of::<u64>() * 2) as u16
);
const_assert!(SYSCALL_SEL.0 & OFFSET_MASK == offset_of!(Gdt, kcode64) as u16);
const_assert!(SYSCALL_SEL.0 & OFFSET_MASK == (offset_of!(Gdt, kdata) - size_of::<u64>()) as u16);

const OFFSET_MASK: u16 = !7;
