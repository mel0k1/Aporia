use core::ops::Range;

use ostd::{mm::vm_space::VmQueriedItem, task::disable_preempt};

use super::{RssDelta, Vmar, util::is_intersected};
use crate::{prelude::*, vm::vmar::is_userspace_vaddr_range};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RemapOldMappingAction {
    Unmap,
    Keep,
}

impl Vmar {
    pub(crate) fn resize_mapping(
        &self,
        map_addr: Vaddr,
        old_size: usize,
        new_size: usize,
        check_single_mapping: bool,
    ) -> Result<()> {
        let mut inner = self.inner.write();
        let mut rss_delta = RssDelta::new(self);

        if check_single_mapping {
            inner.check_lies_in_single_mapping(map_addr, old_size)?;
        } else if inner.vm_mappings.find_one(&map_addr).is_none() {
            return Err(errno!(EFAULT, "there is no mapping at the old address"));
        }

        inner.resize_mapping(self, map_addr, old_size, new_size, &mut rss_delta)
    }

    pub(crate) fn remap(
        &self,
        old_addr: Vaddr,
        old_size: usize,
        new_addr: Option<Vaddr>,
        new_size: usize,
        action: RemapOldMappingAction,
    ) -> Result<Vaddr> {
        debug_assert_eq!(old_addr % PAGE_SIZE, 0);
        debug_assert_eq!(old_size % PAGE_SIZE, 0);
        debug_assert_eq!(new_size % PAGE_SIZE, 0);

        if let Some(new_addr) = new_addr {
            if !new_addr.is_multiple_of(PAGE_SIZE) || !is_userspace_vaddr_range(new_addr, new_size)
            {
                return Err(errno!(
                    EINVAL,
                    "remap: the new range is not aligned or not in userspace"
                ));
            }
            if is_intersected(
                &(old_addr..old_addr.saturating_add(old_size)),
                &(new_addr..new_addr + new_size),
            ) {
                return Err(errno!(
                    EINVAL,
                    "remap: the new range overlaps with the old one"
                ));
            }
        }

        let mut inner = self.inner.write();
        let mut rss_delta = RssDelta::new(self);

        let Some(old_mapping) = inner.vm_mappings.find_one(&old_addr) else {
            return Err(errno!(
                EFAULT,
                "remap: there is no mapping at the old address"
            ));
        };
        if new_size > old_size {
            if !old_mapping.can_expand() {
                return Err(errno!(EFAULT, "remap: device mappings cannot be expanded"));
            }
            if let Some(vmo) = old_mapping.vmo()
                && let offset = vmo.offset() + (old_addr - old_mapping.map_to_addr())
                && offset.checked_add(new_size).is_none()
            {
                return Err(errno!(EINVAL, "remap: the file offset overflows"));
            }
            inner.check_extra_size_fits_rlimit(new_size - old_size)?;
        }

        if old_addr.checked_add(old_size).is_none() {
            return Err(errno!(EINVAL, "remap: the address range overflows"));
        }
        let (old_size, old_range) = if new_size < old_size {
            inner.alloc_free_region_exact_truncate(
                self,
                old_addr + new_size,
                old_size - new_size,
                &mut rss_delta,
            );
            (new_size, old_addr..old_addr + new_size)
        } else {
            (old_size, old_addr..old_addr + old_size)
        };

        let new_range = if let Some(new_addr) = new_addr {
            inner.alloc_free_region_exact_truncate(self, new_addr, new_size, &mut rss_delta)
        } else {
            if action == RemapOldMappingAction::Unmap
                && is_userspace_vaddr_range(old_addr, new_size)
                && inner
                    .alloc_free_region_exact(old_range.end, new_size - old_size)
                    .is_ok()
            {
                let old_mapping_addr = inner.check_lies_in_single_mapping(old_addr, old_size)?;
                let (old_mapping, mut rmap_to_remove) = inner.remove(&old_mapping_addr).unwrap();
                let mut rmap = rmap_to_remove.remove(self, old_mapping_addr);
                let new_mapping = old_mapping.enlarge(new_size - old_size);
                inner.insert_try_merge(self, new_mapping, rmap.as_deref_mut());
                return Ok(old_range.start);
            }

            inner.alloc_free_region(new_size, PAGE_SIZE)?
        };

        let old_mapping_addr = inner.check_lies_in_single_mapping(old_addr, old_size)?;
        if action == RemapOldMappingAction::Unmap {
            let (vm_mapping, mut rmap_to_remove) = inner.remove(&old_mapping_addr).unwrap();
            let mut rmap = rmap_to_remove.remove(self, old_mapping_addr);

            let (left, old_mapping, right) = vm_mapping.split_range(&old_range);
            if let Some(left) = left {
                inner.insert_without_try_merge(self, left, rmap.as_deref_mut());
            }
            if let Some(right) = right {
                inner.insert_without_try_merge(self, right, rmap.as_deref_mut());
            }

            let new_mapping = old_mapping.clone_for_remap_at(new_range.start);
            inner.insert_try_merge(
                self,
                new_mapping.enlarge(new_size - old_size),
                rmap.as_deref_mut(),
            );

            self.move_pt(old_range, new_range.clone());
        } else {
            let old_mapping = inner.vm_mappings.find_one(&old_mapping_addr).unwrap();
            let vmo_for_rmap = old_mapping.vmo_for_rmap().cloned();
            let mut rmap = vmo_for_rmap.as_ref().map(|vmo| vmo.rmap().lock());

            let new_mapping = old_mapping
                .clone_range_for_remap_at(old_addr..old_addr + old_size, new_range.start);
            inner.insert_try_merge(
                self,
                new_mapping.enlarge(new_size - old_size),
                rmap.as_deref_mut(),
            );

            self.move_pt(old_range, new_range.clone());
        }

        Ok(new_range.start)
    }

    fn move_pt(&self, old_range: Range<Vaddr>, new_range: Range<Vaddr>) {
        let preempt_guard = disable_preempt();
        let total_range = old_range.start.min(new_range.start)..old_range.end.max(new_range.end);
        let mut cursor = self
            .vm_space
            .cursor_mut(&preempt_guard, &total_range)
            .unwrap();

        let old_size = old_range.len();
        let mut current_offset = 0;
        while current_offset < old_size {
            cursor.jump(old_range.start + current_offset).unwrap();
            let Some(mapped_va) = cursor.find_next(old_size - current_offset) else {
                break;
            };
            let (va, Some(item)) = cursor.query().unwrap() else {
                panic!("Found mapped page but query failed");
            };
            debug_assert_eq!(mapped_va, va.start);

            let offset = mapped_va - old_range.start;
            let new_map_va = new_range.start + offset;

            match item {
                VmQueriedItem::MappedRam { frame, prop } => {
                    let frame = (*frame).clone();

                    cursor.unmap(PAGE_SIZE);
                    cursor.jump(new_map_va).unwrap();

                    cursor.map(frame, prop);
                }
                VmQueriedItem::MappedIoMem { paddr, prop } => {
                    cursor.unmap(PAGE_SIZE);
                    cursor.jump(new_map_va).unwrap();

                    let (iomem, offset) = cursor.find_iomem_by_paddr(paddr).unwrap();
                    cursor.map_iomem(iomem, prop, PAGE_SIZE, offset);
                }
            }

            current_offset = offset + PAGE_SIZE;
        }

        cursor.flusher().dispatch_tlb_flush();
        cursor.flusher().sync_tlb_flush();
    }
}
