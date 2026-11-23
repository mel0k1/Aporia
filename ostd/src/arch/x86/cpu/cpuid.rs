use core::arch::x86_64::CpuidResult;

use spin::Once;

static MAX_LEAF: Once<u32> = Once::new();
static MAX_HYPERVISOR_LEAF: Once<u32> = Once::new();
static MAX_EXTENDED_LEAF: Once<u32> = Once::new();

#[repr(u32)]
enum Leaf {
    Base = 0x00,
    Xstate = 0x0d,
    Tsc = 0x15,

    HypervisorBase = 0x40000000,
    KvmFeatures = 0x40000001,
    ExtBase = 0x80000000,
}

pub fn cpuid(leaf: u32, subleaf: u32) -> Option<CpuidResult> {
    fn raw_cpuid(leaf: u32, subleaf: u32) -> CpuidResult {
        core::arch::x86_64::__cpuid_count(leaf, subleaf)
    }

    let max_leaf = if leaf < Leaf::HypervisorBase as u32 {
        *MAX_LEAF.call_once(|| raw_cpuid(Leaf::Base as u32, 0).eax)
    } else if leaf < Leaf::ExtBase as u32 {
        *MAX_HYPERVISOR_LEAF.call_once(|| raw_cpuid(Leaf::HypervisorBase as u32, 0).eax)
    } else {
        *MAX_EXTENDED_LEAF.call_once(|| raw_cpuid(Leaf::ExtBase as u32, 0).eax)
    };

    if leaf > max_leaf {
        None
    } else {
        Some(raw_cpuid(leaf, subleaf))
    }
}

pub(in crate::arch) fn query_tsc_freq() -> Option<u64> {
    let CpuidResult {
        eax: denominator,
        ebx: numerator,
        ecx: crystal_freq,
        ..
    } = cpuid(Leaf::Tsc as u32, 0)?;

    if denominator == 0 || numerator == 0 {
        return None;
    }

    if crystal_freq == 0 {
        return None;
    }

    Some((crystal_freq as u64) * (numerator as u64) / (denominator as u64))
}

pub(in crate::arch) fn query_xstate_max_features() -> Option<u64> {
    let res0 = cpuid(Leaf::Xstate as u32, 0)?;
    let res1 = cpuid(Leaf::Xstate as u32, 1)?;

    let xcr_bits = (res0.eax as u64) | ((res0.edx as u64) << 32);
    let xss_bits = (res1.ecx as u64) | ((res1.edx as u64) << 32);

    Some(xcr_bits | xss_bits)
}

pub(in crate::arch) fn query_xsave_area_size() -> Option<u32> {
    cpuid(Leaf::Xstate as u32, 1).map(|res| res.ebx)
}

pub(in crate::arch) fn query_if_running_in_qemu() -> bool {
    let Some(signature) = query_hypervisor_signature() else {
        return false;
    };

    matches!(&signature, b"TCGTCGTCGTCG" | b"KVMKVMKVM\0\0\0")
}

pub(in crate::arch) fn query_if_running_under_kvm() -> bool {
    let Some(signature) = query_hypervisor_signature() else {
        return false;
    };

    matches!(&signature, b"KVMKVMKVM\0\0\0")
}

fn query_hypervisor_signature() -> Option<[u8; 12]> {
    let result = cpuid(Leaf::HypervisorBase as u32, 0)?;

    let mut signature = [0u8; 12];
    signature[0..4].copy_from_slice(&result.ebx.to_ne_bytes());
    signature[4..8].copy_from_slice(&result.ecx.to_ne_bytes());
    signature[8..12].copy_from_slice(&result.edx.to_ne_bytes());

    Some(signature)
}

pub(in crate::arch) fn query_kvm_feature(bit: u32) -> bool {
    cpuid(Leaf::KvmFeatures as u32, 0).is_some_and(|result| result.eax & (1 << bit) != 0)
}
