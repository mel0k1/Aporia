use crate::prelude::*;

#[expect(non_camel_case_types)]
#[expect(dead_code)]
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum AuxKey {
    AT_IGNORE = 1,
    AT_EXECFD = 2,
    AT_PHDR = 3,
    AT_PHENT = 4,
    AT_PHNUM = 5,
    AT_PAGESZ = 6,
    AT_BASE = 7,
    AT_FLAGS = 8,
    AT_ENTRY = 9,
    AT_NOTELF = 10,
    AT_UID = 11,
    AT_EUID = 12,
    AT_GID = 13,
    AT_EGID = 14,
    AT_PLATFORM = 15,
    AT_HWCAP = 16,
    AT_CLKTCK = 17,

    AT_SECURE = 23,
    AT_BASE_PLATFORM = 24,
    AT_RANDOM = 25,
    AT_HWCAP2 = 26,

    AT_EXECFN = 31,
    AT_SYSINFO = 32,
    AT_SYSINFO_EHDR = 33,
}

impl AuxKey {
    pub(super) const AT_NULL: u8 = 0;
}

#[derive(Clone, Debug, Default)]
pub(crate) struct AuxVec {
    table: BTreeMap<AuxKey, u64>,
}

impl AuxVec {
    pub(crate) const fn new() -> AuxVec {
        AuxVec {
            table: BTreeMap::new(),
        }
    }

    pub(crate) fn set(&mut self, key: AuxKey, val: u64) {
        self.table
            .entry(key)
            .and_modify(|val_mut| *val_mut = val)
            .or_insert(val);
    }

    pub(super) fn table(&self) -> &BTreeMap<AuxKey, u64> {
        &self.table
    }
}
