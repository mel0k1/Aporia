use core::num::NonZeroUsize;

use super::{MappedMemory, MappedVmo, RssDelta, VmMapping, Vmar};
use crate::{
    fs::{
        file::{FileLike, MappableObject},
        ramfs::memfd::MemfdInode,
    },
    prelude::*,
    vm::{page_cache::Vmo, perms::VmPerms, vmar::MapHandle},
};

impl Vmar {
    pub(crate) fn new_map<'b>(&self, size: usize, perms: VmPerms) -> VmarMapOptions<'_, 'b> {
        VmarMapOptions::new(self, size, perms)
    }
}

pub(crate) struct VmarMapOptions<'a, 'b> {
    parent: &'a Vmar,
    mappable: Option<MappableObject<'b>>,
    file: Option<Arc<dyn FileLike>>,
    perms: VmPerms,
    may_perms: VmPerms,
    vmo_offset: usize,
    size: usize,
    offset: VmarMapOffset,
    align: usize,
    map_mode: MmapMode,
    handle_page_faults_around: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum VmarMapOffset {
    FixedReplace(usize),
    FixedNoReplace(usize),
    Hint(usize),
    #[cfg(target_arch = "x86_64")]
    Map32Bit(Option<usize>),
    Any,
}

impl<'a, 'b> VmarMapOptions<'a, 'b> {
    fn new(parent: &'a Vmar, size: usize, perms: VmPerms) -> Self {
        Self {
            parent,
            mappable: None,
            file: None,
            perms,
            may_perms: VmPerms::ALL_MAY_PERMS,
            vmo_offset: 0,
            size,
            offset: VmarMapOffset::Any,
            align: PAGE_SIZE,
            map_mode: MmapMode::Private,
            handle_page_faults_around: false,
        }
    }

    pub(crate) fn may_perms(mut self, may_perms: VmPerms) -> Self {
        self.may_perms = may_perms;
        self
    }

    pub(crate) fn vmo(mut self, vmo: Arc<Vmo>) -> Self {
        if self.mappable.is_some() {
            panic!("Cannot set `vmo` when `mappable` is already set");
        }
        self.mappable = Some(MappableObject::Vmo(vmo));

        self
    }

    pub(crate) fn vmo_offset(mut self, offset: usize) -> Self {
        self.vmo_offset = offset;
        self
    }

    pub(crate) fn align(mut self, align: usize) -> Self {
        self.align = align;
        self
    }

    pub(crate) fn offset(mut self, offset: VmarMapOffset) -> Self {
        self.offset = offset;
        self
    }

    pub(crate) fn map_mode(mut self, map_mode: MmapMode) -> Self {
        self.map_mode = map_mode;
        self
    }

    pub(crate) fn handle_page_faults_around(mut self) -> Self {
        self.handle_page_faults_around = true;
        self
    }

    pub(crate) fn mappable(mut self, file: &'b Arc<dyn FileLike>) -> Result<Self> {
        if self.mappable.is_some() {
            panic!("Cannot set `mappable` when `mappable` is already set");
        }
        if self.file.is_some() {
            panic!("Cannot set `mappable` when `file` is already set");
        }

        let mappable = file.mappable(FileMmapRequest::new(self.map_mode.is_shared()))?;
        self.mappable = Some(mappable);
        self.file = Some(file.clone());

        Ok(self)
    }

    pub(crate) fn build(self) -> Result<Vaddr> {
        self.check_options()?;
        let Self {
            parent,
            mappable,
            file,
            perms,
            mut may_perms,
            vmo_offset,
            size: map_size,
            offset,
            align,
            map_mode,
            handle_page_faults_around,
        } = self;

        let mut inner = parent.inner.write();

        inner
            .check_extra_size_fits_rlimit(map_size)
            .or_else(|err| {
                if let VmarMapOffset::FixedReplace(map_to_addr) = offset {
                    let expand_size =
                        map_size - inner.count_overlap_size(map_to_addr..map_to_addr + map_size);
                    inner.check_extra_size_fits_rlimit(expand_size)
                } else {
                    Err(err)
                }
            })?;

        debug!(
            "allocate free region, map_size = 0x{:x}, offset = {:x?}, align = 0x{:x}",
            map_size, offset, align
        );
        let map_to_addr = match offset {
            VmarMapOffset::FixedReplace(map_to_addr) => {
                let mut rss_delta = RssDelta::new(parent);
                inner.alloc_free_region_exact_truncate(
                    parent,
                    map_to_addr,
                    map_size,
                    &mut rss_delta,
                );
                map_to_addr
            }
            VmarMapOffset::FixedNoReplace(map_to_addr) => {
                inner.alloc_free_region_exact(map_to_addr, map_size)?;
                map_to_addr
            }
            VmarMapOffset::Hint(map_to_addr) => {
                if inner.alloc_free_region_exact(map_to_addr, map_size).is_ok() {
                    map_to_addr
                } else {
                    inner.alloc_free_region(map_size, align)?.start
                }
            }
            #[cfg(target_arch = "x86_64")]
            VmarMapOffset::Map32Bit(addr_hint) => {
                use super::{MAP_32BIT_HIGH_LIMIT, VMAR_LOWEST_ADDR};

                if let Some(addr) = addr_hint
                    && (VMAR_LOWEST_ADDR..MAP_32BIT_HIGH_LIMIT).contains(&addr)
                    && map_size <= MAP_32BIT_HIGH_LIMIT - addr
                    && inner.alloc_free_region_exact(addr, map_size).is_ok()
                {
                    addr
                } else {
                    inner.alloc_free_region_below_2gib(map_size, align)?.start
                }
            }
            VmarMapOffset::Any => inner.alloc_free_region(map_size, align)?.start,
        };

        let (mapped_mem, device_mappable) = match mappable {
            Some(MappableObject::Vmo(vmo)) => {
                let path = file.as_ref().map(|file| file.path());

                if let Some(path) = path {
                    debug_assert!(Arc::ptr_eq(&vmo, &path.inode().page_cache().unwrap()));
                }

                let is_writable_tracked = if let Some(path) = path
                    && let Some(memfd_inode) = path.inode().downcast_ref::<MemfdInode>()
                    && map_mode.is_shared()
                    && may_perms.contains(VmPerms::MAY_WRITE)
                {
                    memfd_inode.check_writable(perms, &mut may_perms)?;
                    true
                } else {
                    false
                };

                let mapped_mem =
                    MappedMemory::Vmo(MappedVmo::new(vmo, vmo_offset, is_writable_tracked)?);
                (mapped_mem, None)
            }
            Some(MappableObject::Device(mappable)) => {
                if !map_mode.is_shared() {
                    return Err(errno!(
                        EINVAL,
                        "private device mappings are not yet supported"
                    ));
                }

                (MappedMemory::Anonymous, Some(mappable))
            }
            None => (MappedMemory::Anonymous, None),
        };

        let mut vm_mapping = VmMapping::new(
            NonZeroUsize::new(map_size).unwrap(),
            map_to_addr,
            mapped_mem,
            file,
            map_mode,
            handle_page_faults_around,
            perms | may_perms,
        );

        let vmo_for_rmap = vm_mapping.vmo_for_rmap().cloned();
        let mut rmap = vmo_for_rmap.as_ref().map(|vmo| vmo.rmap().lock());

        if let Some(mappable) = device_mappable {
            let mut rss_delta = RssDelta::new(parent);
            if let Err(err) =
                vm_mapping.populate_device(parent.vm_space(), mappable, vmo_offset, &mut rss_delta)
            {
                rss_delta.add(
                    MapHandle::DEVICE_RSS_TYPE,
                    -(vm_mapping.unmap(parent.vm_space()) as isize),
                );
                return Err(err);
            }
        }

        inner.insert_try_merge(parent, vm_mapping, rmap.as_deref_mut());

        Ok(map_to_addr)
    }

    fn check_options(&self) -> Result<()> {
        debug_assert!(self.align.is_multiple_of(PAGE_SIZE));
        debug_assert!(self.align.is_power_of_two());
        if !self.align.is_multiple_of(PAGE_SIZE) || !self.align.is_power_of_two() {
            return Err(errno!(EINVAL, "invalid align"));
        }
        debug_assert!(self.size.is_multiple_of(self.align));
        if !self.size.is_multiple_of(self.align) {
            return Err(errno!(EINVAL, "invalid mapping size"));
        }
        debug_assert!(self.vmo_offset.is_multiple_of(self.align));
        if !self.vmo_offset.is_multiple_of(self.align) {
            return Err(errno!(EINVAL, "invalid VMO offset"));
        }
        match self.offset {
            VmarMapOffset::FixedReplace(offset)
            | VmarMapOffset::FixedNoReplace(offset)
            | VmarMapOffset::Hint(offset) => {
                debug_assert!(offset.is_multiple_of(self.align));
                if !offset.is_multiple_of(self.align) {
                    return Err(errno!(EINVAL, "invalid offset"));
                }
            }
            #[cfg(target_arch = "x86_64")]
            VmarMapOffset::Map32Bit(None) => (),
            #[cfg(target_arch = "x86_64")]
            VmarMapOffset::Map32Bit(Some(offset)) => {
                debug_assert!(offset.is_multiple_of(self.align));
                if !offset.is_multiple_of(self.align) {
                    return Err(errno!(EINVAL, "invalid offset"));
                }
            }
            VmarMapOffset::Any => (),
        }
        self.check_perms()
    }

    fn check_perms(&self) -> Result<()> {
        if !VmPerms::ALL_MAY_PERMS.contains(self.may_perms)
            || !VmPerms::ALL_PERMS.contains(self.perms)
        {
            return Err(errno!(EACCES, "invalid perms"));
        }

        let vm_perms = self.perms | self.may_perms;
        vm_perms.check()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileMmapRequest {
    is_shared: bool,
}

impl FileMmapRequest {
    pub(crate) const fn new(is_shared: bool) -> Self {
        Self { is_shared }
    }

    pub const fn is_shared(self) -> bool {
        self.is_shared
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MmapMode {
    Private,
    Shared,
}

impl MmapMode {
    pub(crate) const fn is_shared(self) -> bool {
        matches!(self, Self::Shared)
    }
}
