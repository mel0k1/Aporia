use core::ops::Range;

use super::{Interval, Vmar, util::get_intersected_range};
use crate::{prelude::*, vm::perms::VmPerms};

impl Vmar {
    pub(crate) fn protect(&self, perms: VmPerms, range: Range<usize>) -> Result<()> {
        debug_assert!(range.start.is_multiple_of(PAGE_SIZE));
        debug_assert!(range.end.is_multiple_of(PAGE_SIZE));

        let mut inner = self.inner.write();

        let mut protect_mappings = Vec::new();

        for vm_mapping in inner.vm_mappings.find(&range) {
            protect_mappings.push((vm_mapping.range(), vm_mapping.perms()))
        }

        let mut last_mapping_end = range.start;
        for (vm_mapping_range, vm_mapping_perms) in protect_mappings {
            if last_mapping_end < vm_mapping_range.start {
                return Err(errno!(
                    ENOMEM,
                    "the range contains pages that are not mapped"
                ));
            }
            last_mapping_end = vm_mapping_range.end;

            if perms == vm_mapping_perms & VmPerms::ALL_PERMS {
                continue;
            }
            let new_perms = perms | (vm_mapping_perms & VmPerms::ALL_MAY_PERMS);
            new_perms.check()?;

            let Some((vm_mapping, mut rmap_to_remove)) = inner.remove(&vm_mapping_range.start)
            else {
                continue;
            };
            let mut rmap = rmap_to_remove.remove(self, vm_mapping_range.start);

            let vm_mapping_range = vm_mapping.range();
            let intersected_range = get_intersected_range(&range, &vm_mapping_range);

            let (left, taken, right) = vm_mapping.split_range(&intersected_range);

            if let Some(left) = left {
                inner.insert_without_try_merge(self, left, rmap.as_deref_mut());
            }
            if let Some(right) = right {
                inner.insert_without_try_merge(self, right, rmap.as_deref_mut());
            }

            let taken = taken.protect(self.vm_space(), new_perms);
            inner.insert_try_merge(self, taken, rmap.as_deref_mut());
        }

        if last_mapping_end < range.end {
            return Err(errno!(
                ENOMEM,
                "the range contains pages that are not mapped"
            ));
        }

        Ok(())
    }
}
