mod access_alien;
mod fork;
pub(super) mod map;
pub(super) mod page_fault;
mod protect;
mod query;
pub(super) mod remap;
mod unmap;

use core::{
    array,
    ops::Range,
    sync::atomic::{AtomicUsize, Ordering},
};

use align_ext::AlignExt;
use aster_util::per_cpu_counter::PerCpuCounter;
use ostd::{cpu::CpuId, mm::VmSpace};

use super::{
    Rmap, RmapEntry, VMAR_CAP_ADDR, VMAR_LOWEST_ADDR,
    interval_set::{Interval, IntervalSet},
    is_userspace_vaddr, is_userspace_vaddr_range,
    util::{self, get_intersected_range},
    vm_mapping::{MappedMemory, MappedVmo, VmMapping},
};
use crate::{
    prelude::*,
    process::{INIT_STACK_SIZE, Process, ProcessVm, ResourceType},
    vm::page_cache::Vmo,
};

#[cfg(target_arch = "x86_64")]
const MAP_32BIT_HIGH_LIMIT: Vaddr = 0x8000_0000;

pub(crate) struct Vmar {
    inner: RwMutex<VmarInner>,
    vm_space: Arc<VmSpace>,
    rss_counters: [PerCpuCounter; NUM_RSS_COUNTERS],
    process_vm: ProcessVm,
    num_handles: AtomicUsize,
    weak_self: Weak<Self>,
}

impl Vmar {
    pub(super) fn new(process_vm: ProcessVm) -> Arc<Self> {
        let inner = VmarInner::new();
        let vm_space = VmSpace::new();
        let rss_counters = array::from_fn(|_| PerCpuCounter::new());
        Arc::new_cyclic(move |weak_self| Vmar {
            inner: RwMutex::new(inner),
            vm_space: Arc::new(vm_space),
            rss_counters,
            process_vm,
            num_handles: AtomicUsize::new(1),
            weak_self: weak_self.clone(),
        })
    }

    pub(crate) fn get_rss_counter(&self, rss_type: RssType) -> usize {
        self.rss_counters[rss_type as usize].sum_all_cpus()
    }

    pub(crate) fn get_mappings_total_size(&self) -> usize {
        self.inner.read().total_vm
    }

    pub(crate) fn vm_space(&self) -> &Arc<VmSpace> {
        &self.vm_space
    }

    pub(crate) fn process_vm(&self) -> &ProcessVm {
        &self.process_vm
    }

    pub(crate) fn has_multiple_handles(&self) -> bool {
        self.num_handles.load(Ordering::Relaxed) > 1
    }

    pub(super) fn inc_num_handles(&self) {
        let old_num_handles = self.num_handles.fetch_add(1, Ordering::Relaxed);
        debug_assert_ne!(old_num_handles, 0);
    }

    pub(super) fn dec_num_handles(&self) {
        let old_num_handles = self.num_handles.fetch_sub(1, Ordering::Relaxed);
        debug_assert_ne!(old_num_handles, 0);
        if old_num_handles == 1 {
            self.clear();
        }
    }

    fn add_rss_counter(&self, rss_type: RssType, val: isize) {
        let cpu_id = CpuId::current_racy();
        self.rss_counters[rss_type as usize].add_on_cpu(cpu_id, val);
    }
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub(crate) enum RssType {
    File = 0,
    Anon = 1,
}

const NUM_RSS_COUNTERS: usize = 2;

pub(super) struct RssDelta<'a> {
    delta: [isize; NUM_RSS_COUNTERS],
    operated_vmar: &'a Vmar,
}

impl<'a> RssDelta<'a> {
    pub(super) fn new(operated_vmar: &'a Vmar) -> Self {
        Self {
            delta: [0; NUM_RSS_COUNTERS],
            operated_vmar,
        }
    }

    pub(super) fn add(&mut self, rss_type: RssType, increment: isize) {
        self.delta[rss_type as usize] += increment;
    }

    fn get(&self, rss_type: RssType) -> isize {
        self.delta[rss_type as usize]
    }
}

impl Drop for RssDelta<'_> {
    fn drop(&mut self) {
        for i in 0..NUM_RSS_COUNTERS {
            let rss_type = RssType::try_from(i as u32).unwrap();
            let delta = self.get(rss_type);
            self.operated_vmar.add_rss_counter(rss_type, delta);
        }
    }
}

#[must_use]
struct RmapToRemove {
    vmo: Option<Arc<Vmo>>,
}

impl RmapToRemove {
    pub(self) fn new(vmo: Option<Arc<Vmo>>) -> Self {
        Self { vmo }
    }

    pub(self) fn remove(&mut self, vmar: &Vmar, addr: Vaddr) -> Option<MutexGuard<'_, Rmap>> {
        let mut rmap = self.vmo.as_ref()?.rmap().lock();
        rmap.remove(vmar.weak_self.clone(), addr);
        Some(rmap)
    }
}

struct VmarInner {
    vm_mappings: IntervalSet<Vaddr, VmMapping>,
    total_vm: usize,
}

impl VmarInner {
    const fn new() -> Self {
        Self {
            vm_mappings: IntervalSet::new(),
            total_vm: 0,
        }
    }

    fn check_extra_size_fits_rlimit(&self, expand_size: usize) -> Result<()> {
        let Some(process) = Process::current() else {
            return Ok(());
        };

        let rlimit_as = process
            .resource_limits()
            .get_rlimit(ResourceType::RLIMIT_AS)
            .get_cur();

        if rlimit_as.saturating_sub(self.total_vm as u64) < expand_size as u64 {
            return Err(errno!(ENOMEM, "the address space size limit is reached"));
        }
        Ok(())
    }

    fn check_lies_in_single_mapping(&self, addr: Vaddr, size: usize) -> Result<Vaddr> {
        if let Some(vm_mapping) = self
            .vm_mappings
            .find_one(&addr)
            .filter(|vm_mapping| vm_mapping.map_end() - addr >= size)
        {
            Ok(vm_mapping.map_to_addr())
        } else {
            Err(errno!(EFAULT, "the range must lie in a single mapping"))
        }
    }

    fn insert_without_try_merge(
        &mut self,
        vmar: &Vmar,
        vm_mapping: VmMapping,
        rmap: Option<&mut Rmap>,
    ) {
        self.total_vm += vm_mapping.map_size();

        if let Some(rmap) = rmap {
            rmap.insert(
                vmar.weak_self.clone(),
                RmapEntry {
                    vaddr: vm_mapping.map_to_addr(),
                    offset: vm_mapping.vmo().unwrap().offset(),
                    size: vm_mapping.map_size(),
                },
            );
        }
        self.vm_mappings.insert(vm_mapping);
    }

    fn insert_try_merge(
        &mut self,
        vmar: &Vmar,
        vm_mapping: VmMapping,
        mut rmap: Option<&mut Rmap>,
    ) {
        self.total_vm += vm_mapping.map_size();
        let mut vm_mapping = vm_mapping;
        let addr = vm_mapping.map_to_addr();

        if let Some(prev) = self.vm_mappings.find_prev(&addr) {
            let (new_mapping, to_remove) = vm_mapping.try_merge_with(prev);
            vm_mapping = new_mapping;
            if let Some(addr) = to_remove {
                self.vm_mappings.remove(&addr);
                if let Some(rmap) = rmap.as_deref_mut() {
                    rmap.remove(vmar.weak_self.clone(), addr);
                }
            }
        }

        if let Some(next) = self.vm_mappings.find_next(&addr) {
            let (new_mapping, to_remove) = vm_mapping.try_merge_with(next);
            vm_mapping = new_mapping;
            if let Some(addr) = to_remove {
                self.vm_mappings.remove(&addr);
                if let Some(rmap) = rmap.as_deref_mut() {
                    rmap.remove(vmar.weak_self.clone(), addr);
                }
            }
        }

        if let Some(rmap) = rmap {
            rmap.insert(
                vmar.weak_self.clone(),
                RmapEntry {
                    vaddr: vm_mapping.map_to_addr(),
                    offset: vm_mapping.vmo().unwrap().offset(),
                    size: vm_mapping.map_size(),
                },
            );
        }
        self.vm_mappings.insert(vm_mapping);
    }

    fn remove(&mut self, key: &Vaddr) -> Option<(VmMapping, RmapToRemove)> {
        let vm_mapping = self.vm_mappings.remove(key)?;
        self.total_vm -= vm_mapping.map_size();

        let rmap_to_remove = RmapToRemove::new(vm_mapping.vmo_for_rmap().cloned());

        Some((vm_mapping, rmap_to_remove))
    }

    fn query(&self, range: &Range<Vaddr>) -> impl Iterator<Item = &VmMapping> {
        self.vm_mappings.find(range)
    }

    fn count_overlap_size(&self, range: Range<Vaddr>) -> usize {
        let mut sum_overlap_size = 0;
        for vm_mapping in self.vm_mappings.find(&range) {
            let vm_mapping_range = vm_mapping.range();
            let intersected_range = get_intersected_range(&range, &vm_mapping_range);
            sum_overlap_size += intersected_range.end - intersected_range.start;
        }
        sum_overlap_size
    }

    fn alloc_free_region_exact(&mut self, offset: Vaddr, size: usize) -> Result<Range<Vaddr>> {
        if self
            .vm_mappings
            .find(&(offset..offset + size))
            .next()
            .is_some()
        {
            return Err(errno!(
                EEXIST,
                "the range contains pages that are already mapped"
            ));
        }

        Ok(offset..(offset + size))
    }

    fn alloc_free_region_exact_truncate(
        &mut self,
        vmar: &Vmar,
        offset: Vaddr,
        size: usize,
        rss_delta: &mut RssDelta,
    ) -> Range<Vaddr> {
        let range = offset..offset + size;

        let mut mappings_to_remove = Vec::new();
        for vm_mapping in self.vm_mappings.find(&range) {
            mappings_to_remove.push(vm_mapping.map_to_addr());
        }

        for vm_mapping_addr in mappings_to_remove {
            let (vm_mapping, mut rmap_to_remove) = self.remove(&vm_mapping_addr).unwrap();
            let mut rmap = rmap_to_remove.remove(vmar, vm_mapping_addr);

            let vm_mapping_range = vm_mapping.range();
            let intersected_range = get_intersected_range(&range, &vm_mapping_range);

            let (left, taken, right) = vm_mapping.split_range(&intersected_range);
            if let Some(left) = left {
                self.insert_without_try_merge(vmar, left, rmap.as_deref_mut());
            }
            if let Some(right) = right {
                self.insert_without_try_merge(vmar, right, rmap.as_deref_mut());
            }

            drop(rmap);

            rss_delta.add(taken.rss_type(), -(taken.unmap(&vmar.vm_space) as isize));
        }

        range
    }

    fn alloc_free_region(&mut self, size: usize, align: usize) -> Result<Range<Vaddr>> {
        let high_limit = VMAR_CAP_ADDR - INIT_STACK_SIZE - PAGE_SIZE * 2048;
        let low_limit = VMAR_LOWEST_ADDR;
        self.alloc_free_region_in_range(size, align, low_limit, high_limit)
    }

    #[cfg(target_arch = "x86_64")]
    fn alloc_free_region_below_2gib(&mut self, size: usize, align: usize) -> Result<Range<Vaddr>> {
        let high_limit = MAP_32BIT_HIGH_LIMIT.min(VMAR_CAP_ADDR);
        let low_limit = VMAR_LOWEST_ADDR;
        self.alloc_free_region_in_range(size, align, low_limit, high_limit)
    }

    fn alloc_free_region_in_range(
        &mut self,
        size: usize,
        align: usize,
        low_limit: Vaddr,
        high_limit: Vaddr,
    ) -> Result<Range<Vaddr>> {
        fn try_alloc_in_hole(
            hole_start: Vaddr,
            hole_end: Vaddr,
            size: usize,
            align: usize,
        ) -> Option<Range<Vaddr>> {
            let start = hole_end.checked_sub(size)?.align_down(align);
            if start >= hole_start {
                Some(start..start + size)
            } else {
                None
            }
        }

        let mut prev_vm_mapping_start = high_limit;
        for vm_mapping in self.vm_mappings.iter().rev() {
            let hole_start = vm_mapping.range().end.max(low_limit);
            let hole_end = prev_vm_mapping_start.min(high_limit);

            if let Some(region) = try_alloc_in_hole(hole_start, hole_end, size, align) {
                return Ok(region);
            }

            prev_vm_mapping_start = vm_mapping.range().start;
            if prev_vm_mapping_start <= low_limit {
                break;
            }
        }

        if prev_vm_mapping_start > low_limit {
            let hole_start = low_limit;
            let hole_end = prev_vm_mapping_start.min(high_limit);
            if let Some(region) = try_alloc_in_hole(hole_start, hole_end, size, align) {
                return Ok(region);
            }
        }

        Err(errno!(ENOMEM, "no free region for mapping can be found"))
    }

    fn resize_mapping(
        &mut self,
        vmar: &Vmar,
        map_addr: Vaddr,
        old_size: usize,
        new_size: usize,
        rss_delta: &mut RssDelta,
    ) -> Result<()> {
        debug_assert_eq!(map_addr % PAGE_SIZE, 0);
        debug_assert_eq!(old_size % PAGE_SIZE, 0);
        debug_assert_eq!(new_size % PAGE_SIZE, 0);

        if new_size == 0 {
            return Err(errno!(EINVAL, "resizing a mapping to zero is invalid"));
        }

        if new_size == old_size {
            return Ok(());
        }

        if !is_userspace_vaddr_range(map_addr, new_size) {
            return Err(errno!(EINVAL, "the address range is not in userspace"));
        }

        if new_size < old_size {
            if map_addr.checked_add(old_size).is_none() {
                return Err(errno!(EINVAL, "the address range overflows"));
            }
            self.alloc_free_region_exact_truncate(
                vmar,
                map_addr + new_size,
                old_size - new_size,
                rss_delta,
            );
            return Ok(());
        }

        let old_map_end = map_addr + old_size;
        self.alloc_free_region_exact(old_map_end, new_size - old_size)
            .map_err(|_| errno!(ENOMEM, "the range contains pages that are already mapped",))?;

        let last_mapping = self.vm_mappings.find_one(&(old_map_end - 1)).unwrap();
        let last_mapping_addr = last_mapping.map_to_addr();
        debug_assert_eq!(last_mapping.map_end(), old_map_end);

        if !last_mapping.can_expand() {
            return Err(errno!(EFAULT, "device mappings cannot be expanded"));
        }

        self.check_extra_size_fits_rlimit(new_size - old_size)?;
        let (last_mapping, mut rmap_to_remove) = self.remove(&last_mapping_addr).unwrap();
        let mut rmap = rmap_to_remove.remove(vmar, last_mapping_addr);
        let last_mapping = last_mapping.enlarge(new_size - old_size);
        self.insert_try_merge(vmar, last_mapping, rmap.as_deref_mut());
        Ok(())
    }
}
