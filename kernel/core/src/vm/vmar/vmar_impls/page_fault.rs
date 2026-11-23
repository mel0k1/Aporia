#![short_vis_path::add(vmar)]

use super::{Interval, RssDelta, Vmar};
use crate::{prelude::*, vm::perms::VmPerms};

impl Vmar {
    pub(crate) fn handle_page_fault(&self, page_fault_info: &PageFaultInfo) -> Result<()> {
        let inner = self.inner.read();

        let address = page_fault_info.address;
        if let Some(vm_mapping) = inner.vm_mappings.find_one(&address) {
            debug_assert!(vm_mapping.range().contains(&address));

            let mut rss_delta = RssDelta::new(self);
            return vm_mapping.handle_page_fault(&self.vm_space, page_fault_info, &mut rss_delta);
        }

        Err(errno!(
            EACCES,
            "no VM mappings contain the page fault address"
        ))
    }
}

#[derive(Debug)]
pub(crate) struct PageFaultInfo {
    pub(in vmar) address: Vaddr,

    pub(in vmar) required_perms: VmPerms,

    is_forced: bool,
}

impl PageFaultInfo {
    pub(crate) fn new(address: Vaddr, required_perms: VmPerms) -> Self {
        Self {
            address,
            required_perms,
            is_forced: false,
        }
    }

    pub(in vmar) fn is_forced(&self) -> bool {
        self.is_forced
    }

    pub(super) fn force(mut self) -> Self {
        self.is_forced = true;
        self
    }
}
