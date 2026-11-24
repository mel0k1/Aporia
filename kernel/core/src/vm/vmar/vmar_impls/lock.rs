use core::ops::Range;

use ostd::task::disable_preempt;

use super::{
    Interval, RssDelta, Vmar, VmarInner, page_fault::PageFaultInfo, util::get_intersected_range,
};
use crate::{prelude::*, vm::perms::VmPerms};

/// The metadata of a mapping that is relevant for locking.
struct MappingInfo {
    range: Range<Vaddr>,
    is_locked: bool,
    is_lock_onfault: bool,
}

impl Vmar {
    /// Locks the pages in the specified range in memory (`mlock`/`mlock2`).
    ///
    /// All pages in the range must be mapped, otherwise an [`ENOMEM`] error
    /// will be returned. Note that pages before an unmapped hole are still
    /// locked.
    ///
    /// Unless `lock_onfault` is specified, the locked pages are also
    /// populated (i.e., resident) after this call. Locked memory is accounted
    /// against `RLIMIT_MEMLOCK` of the current process.
    ///
    /// [`ENOMEM`]: Errno::ENOMEM
    pub(crate) fn lock(&self, range: Range<usize>, lock_onfault: bool) -> Result<()> {
        debug_assert!(range.start.is_multiple_of(PAGE_SIZE));
        debug_assert!(range.end.is_multiple_of(PAGE_SIZE));

        let mut inner = self.inner.write();

        let affected_mappings = self.collect_affected_mappings(&inner, &range)?;

        // Check whether the newly locked memory still fits within
        // `RLIMIT_MEMLOCK` before doing any modification.
        let mut newly_locked_size = 0;
        for mapping in &affected_mappings {
            if !mapping.is_locked {
                let intersected_range = get_intersected_range(&range, &mapping.range);
                newly_locked_size += intersected_range.end - intersected_range.start;
            }
        }
        inner.check_locked_size_fits_rlimit(newly_locked_size)?;

        let mut to_populate = Vec::new();
        for mapping in affected_mappings {
            let intersected_range = get_intersected_range(&range, &mapping.range);

            if mapping.is_locked && mapping.is_lock_onfault == lock_onfault {
                // The intersected pages are already locked in the same mode.
                continue;
            }

            let Some((vm_mapping, mut rmap_to_remove)) = inner.remove(&mapping.range.start) else {
                continue;
            };
            let mut rmap = rmap_to_remove.remove(self, mapping.range.start);

            let (left, taken, right) = vm_mapping.split_range(&intersected_range);

            if let Some(left) = left {
                inner.insert_without_try_merge(self, left, rmap.as_deref_mut());
            }
            if let Some(right) = right {
                inner.insert_without_try_merge(self, right, rmap.as_deref_mut());
            }

            let mut taken = taken;
            taken.set_locked(true, lock_onfault);

            // The pages need population if they were not locked before, or if
            // they were locked in the "lock on fault" mode and are now
            // upgraded to a full lock.
            if (!mapping.is_locked || mapping.is_lock_onfault) && !lock_onfault {
                to_populate.push(intersected_range);
            }

            inner.insert_try_merge(self, taken, rmap.as_deref_mut());
        }

        drop(inner);

        for populate_range in to_populate {
            self.populate_range(populate_range)?;
        }

        Ok(())
    }

    /// Unlocks the pages in the specified range (`munlock`).
    ///
    /// The range need not correspond to a previously locked range; any pages
    /// in the range that are locked will be unlocked. All pages in the range
    /// must be mapped, otherwise an [`ENOMEM`] error will be returned.
    ///
    /// [`ENOMEM`]: Errno::ENOMEM
    pub(crate) fn unlock(&self, range: Range<usize>) -> Result<()> {
        debug_assert!(range.start.is_multiple_of(PAGE_SIZE));
        debug_assert!(range.end.is_multiple_of(PAGE_SIZE));

        let mut inner = self.inner.write();

        let affected_mappings = self.collect_affected_mappings(&inner, &range)?;

        for mapping in affected_mappings {
            if !mapping.is_locked {
                continue;
            }
            let intersected_range = get_intersected_range(&range, &mapping.range);

            let Some((vm_mapping, mut rmap_to_remove)) = inner.remove(&mapping.range.start) else {
                continue;
            };
            let mut rmap = rmap_to_remove.remove(self, mapping.range.start);

            let (left, taken, right) = vm_mapping.split_range(&intersected_range);

            if let Some(left) = left {
                inner.insert_without_try_merge(self, left, rmap.as_deref_mut());
            }
            if let Some(right) = right {
                inner.insert_without_try_merge(self, right, rmap.as_deref_mut());
            }

            let mut taken = taken;
            taken.set_locked(false, false);
            inner.insert_try_merge(self, taken, rmap.as_deref_mut());
        }

        Ok(())
    }

    /// Locks all pages that are currently mapped in the VMAR
    /// (`mlockall` with `MCL_CURRENT`).
    ///
    /// Unless `lock_onfault` is specified, all locked pages are also
    /// populated after this call. If the total size of mapped memory exceeds
    /// `RLIMIT_MEMLOCK`, an [`ENOMEM`] error will be returned and no mapping
    /// will be locked.
    ///
    /// [`ENOMEM`]: Errno::ENOMEM
    pub(crate) fn lock_all(&self, lock_onfault: bool) -> Result<()> {
        let mut inner = self.inner.write();

        let mut mapping_infos = Vec::new();
        let mut total_size = 0;
        for vm_mapping in inner.vm_mappings.iter() {
            let range = vm_mapping.range();
            total_size += range.end - range.start;
            mapping_infos.push(MappingInfo {
                range,
                is_locked: vm_mapping.is_locked(),
                is_lock_onfault: vm_mapping.is_lock_onfault(),
            });
        }
        let already_locked_size = inner.total_locked_size();
        inner.check_locked_size_fits_rlimit(total_size - already_locked_size)?;

        let mut to_populate = Vec::new();
        for mapping in mapping_infos {
            if mapping.is_locked && mapping.is_lock_onfault == lock_onfault {
                continue;
            }

            let Some((mut vm_mapping, mut rmap_to_remove)) = inner.remove(&mapping.range.start)
            else {
                continue;
            };
            let mut rmap = rmap_to_remove.remove(self, mapping.range.start);

            vm_mapping.set_locked(true, lock_onfault);
            inner.insert_try_merge(self, vm_mapping, rmap.as_deref_mut());

            if !mapping.is_locked && !lock_onfault {
                to_populate.push(mapping.range);
            }
        }

        drop(inner);

        for populate_range in to_populate {
            self.populate_range(populate_range)?;
        }

        Ok(())
    }

    /// Unlocks all pages that are currently mapped in the VMAR and disables
    /// locking of future mappings (`munlockall`).
    pub(crate) fn unlock_all(&self) -> Result<()> {
        let mut inner = self.inner.write();

        let mut mapping_addrs = Vec::new();
        for vm_mapping in inner.vm_mappings.iter() {
            mapping_addrs.push(vm_mapping.range().start);
        }

        for mapping_addr in mapping_addrs {
            let Some((mut vm_mapping, mut rmap_to_remove)) = inner.remove(&mapping_addr) else {
                continue;
            };
            let mut rmap = rmap_to_remove.remove(self, mapping_addr);

            vm_mapping.set_locked(false, false);
            inner.insert_try_merge(self, vm_mapping, rmap.as_deref_mut());
        }

        inner.set_lock_future(false, false);

        Ok(())
    }

    /// Sets whether mappings created in the future should be locked in
    /// memory (`mlockall` with `MCL_FUTURE`).
    pub(crate) fn set_lock_future(&self, lock_future: bool, lock_future_onfault: bool) {
        self.inner
            .write()
            .set_lock_future(lock_future, lock_future_onfault);
    }

    /// Collects the metadata of the mappings that intersect with the range,
    /// checking that the range contains no unmapped holes.
    fn collect_affected_mappings(
        &self,
        inner: &VmarInner,
        range: &Range<Vaddr>,
    ) -> Result<Vec<MappingInfo>> {
        let mut affected_mappings = Vec::new();

        let mut last_mapping_end = range.start;
        for vm_mapping in inner.vm_mappings.find(range) {
            let vm_mapping_range = vm_mapping.range();
            if last_mapping_end < vm_mapping_range.start {
                return Err(errno!(
                    ENOMEM,
                    "the range contains pages that are not mapped"
                ));
            }
            last_mapping_end = vm_mapping_range.end;

            affected_mappings.push(MappingInfo {
                range: vm_mapping_range,
                is_locked: vm_mapping.is_locked(),
                is_lock_onfault: vm_mapping.is_lock_onfault(),
            });
        }
        if last_mapping_end < range.end {
            return Err(errno!(
                ENOMEM,
                "the range contains pages that are not mapped"
            ));
        }

        Ok(affected_mappings)
    }

    /// Populates the pages in the specified range by synthesizing page faults.
    ///
    /// Pages that are already present in the page table (including eagerly
    /// populated device pages) are skipped, and so are pages that are not
    /// mapped (e.g., unmapped concurrently).
    pub(super) fn populate_range(&self, range: Range<Vaddr>) -> Result<()> {
        let inner = self.inner.read();

        let mut rss_delta = RssDelta::new(self);
        for page in (range.start..range.end).step_by(PAGE_SIZE) {
            let preempt_guard = disable_preempt();
            let mut cursor = self
                .vm_space
                .cursor(&preempt_guard, &(page..page + PAGE_SIZE))?;
            let is_present = matches!(cursor.query(), Ok((_, Some(_))));
            drop(cursor);
            drop(preempt_guard);
            if is_present {
                continue;
            }

            let Some(vm_mapping) = inner.vm_mappings.find_one(&page) else {
                continue;
            };

            let mut required_perms = VmPerms::READ;
            if vm_mapping.perms().contains(VmPerms::WRITE) {
                required_perms |= VmPerms::WRITE;
            }
            let page_fault_info = PageFaultInfo::new(page, required_perms);
            vm_mapping.handle_page_fault(&self.vm_space, &page_fault_info, &mut rss_delta)?;
        }

        Ok(())
    }
}
