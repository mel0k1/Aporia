mod ept;

pub(crate) use ept::{PageTableEntry, PagingConsts};

use super::vmx::{VmxGuard, invept};
use crate::{
    mm::{Frame, frame::meta::AnyFrameMeta},
    prelude::*,
    smp::PendingIpis,
    sync::RcuDrop,
};

pub(crate) struct ArchGuestPhysMemSpace {
    vmx_guard: VmxGuard,
}

impl ArchGuestPhysMemSpace {
    pub(crate) fn new() -> Result<Self> {
        Ok(Self {
            vmx_guard: VmxGuard::acquire_vmx()?,
        })
    }

    pub(crate) fn invalidate(&self, frames: &[RcuDrop<Frame<dyn AnyFrameMeta>>]) -> PendingIpis {
        invept::invalidate(&self.vmx_guard, frames)
    }
}
