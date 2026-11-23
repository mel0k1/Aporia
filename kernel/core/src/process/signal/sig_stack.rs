use crate::{prelude::*, process::signal::c_types::stack_t};

#[derive(Debug, Default)]
pub(crate) struct SigStack {
    base: Vaddr,
    flags: SigStackFlags,
    size: usize,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum SigStackStatus {
    Inactive,
    Active,
    Disable,
}

bitflags! {
    #[derive(Clone, Copy, Debug, Default)]
    pub(crate) struct SigStackFlags: u32 {
        const SS_ONSTACK    = 1 << 0;
        const SS_DISABLE    = 1 << 1;
        const SS_AUTODISARM = 1 << 31;
    }
}

impl SigStack {
    pub(crate) fn new(base: Vaddr, flags: SigStackFlags, size: usize) -> Self {
        Self { base, flags, size }
    }

    pub(crate) fn base(&self) -> Vaddr {
        self.base
    }

    pub(crate) fn flags(&self) -> SigStackFlags {
        self.flags
    }

    pub(crate) fn active_status(&self, sp: usize) -> SigStackStatus {
        if self.size == 0 {
            return SigStackStatus::Disable;
        }

        if self.contains(sp) {
            return SigStackStatus::Active;
        }

        SigStackStatus::Inactive
    }

    pub(crate) fn size(&self) -> usize {
        self.size
    }

    pub(crate) fn contains(&self, sp: usize) -> bool {
        if self.flags().contains(SigStackFlags::SS_AUTODISARM) {
            return false;
        }

        self.base < sp && sp <= self.base + self.size
    }

    pub(super) fn reset(&mut self) {
        self.base = 0;
        self.size = 0;
        self.flags = SigStackFlags::SS_DISABLE;
    }
}

impl From<&SigStack> for stack_t {
    fn from(value: &SigStack) -> Self {
        Self {
            ss_sp: value.base,
            ss_flags: value.flags.bits() as _,
            ss_size: value.size,
            ..Default::default()
        }
    }
}
