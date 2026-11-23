use x86::msr::{IA32_PAT, wrmsr};

use super::PteFlags;
use crate::{const_assert, mm::page_prop::CachePolicy};

const IA32_PAT_MAPPINGS: [CachePolicy; 8] = [
    CachePolicy::Writeback,
    CachePolicy::Writethrough,
    CachePolicy::WriteCombining,
    CachePolicy::Uncacheable,
    CachePolicy::Writeback,
    CachePolicy::Writethrough,
    CachePolicy::WriteCombining,
    CachePolicy::Uncacheable,
];

pub(super) const fn flags_to_cache_policy(flags: PteFlags) -> CachePolicy {
    let bits = flags.bits();
    let mut index = 0usize;
    if bits & PteFlags::NO_CACHE.bits() != 0 {
        index |= 2;
    }
    if bits & PteFlags::WRITE_THROUGH.bits() != 0 {
        index |= 1;
    }
    IA32_PAT_MAPPINGS[index]
}

pub(super) const fn cache_policy_to_flags(cache_policy: CachePolicy) -> PteFlags {
    let bits = match cache_policy {
        CachePolicy::Writeback => 0,
        CachePolicy::Writethrough => PteFlags::WRITE_THROUGH.bits(),
        CachePolicy::Uncacheable => PteFlags::NO_CACHE.bits() | PteFlags::WRITE_THROUGH.bits(),
        CachePolicy::WriteCombining => PteFlags::NO_CACHE.bits(),
        _ => panic!("unsupported cache policy"),
    };
    PteFlags::from_bits_truncate(bits)
}

const_assert!(matches!(
    flags_to_cache_policy(cache_policy_to_flags(CachePolicy::Writeback)),
    CachePolicy::Writeback
));
const_assert!(matches!(
    flags_to_cache_policy(cache_policy_to_flags(CachePolicy::Writethrough)),
    CachePolicy::Writethrough
));
const_assert!(matches!(
    flags_to_cache_policy(cache_policy_to_flags(CachePolicy::Uncacheable)),
    CachePolicy::Uncacheable
));
const_assert!(matches!(
    flags_to_cache_policy(cache_policy_to_flags(CachePolicy::WriteCombining)),
    CachePolicy::WriteCombining
));

pub(super) fn configure_pat() {
    fn cache_policy_to_pat_entry(cache_policy: CachePolicy) -> u8 {
        match cache_policy {
            CachePolicy::Uncacheable => 0x00,
            CachePolicy::WriteCombining => 0x01,
            CachePolicy::WriteProtected => 0x05,
            CachePolicy::Writethrough => 0x04,
            CachePolicy::Writeback => 0x06,
        }
    }

    let mut programmed_pat = 0u64;
    for (idx, policy) in IA32_PAT_MAPPINGS.iter().copied().enumerate() {
        programmed_pat |= (cache_policy_to_pat_entry(policy) as u64) << (idx * 8);
    }

    unsafe {
        wrmsr(IA32_PAT, programmed_pat);
    }
}
