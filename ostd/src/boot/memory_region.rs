use core::ops::Deref;

use align_ext::AlignExt;

use crate::mm::{PAGE_SIZE, Paddr, Vaddr, kspace::kernel_loaded_offset};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum MemoryRegionType {
    BadMemory = 0,
    Unknown = 1,
    NonVolatileSleep = 2,
    Reserved = 3,
    Kernel = 4,
    Module = 5,
    Framebuffer = 6,
    Reclaimable = 7,
    Usable = 8,
}

impl MemoryRegionType {
    pub fn is_physical(self) -> bool {
        !matches!(
            self,
            Self::BadMemory | Self::Unknown | Self::Reserved | Self::Framebuffer
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct MemoryRegion {
    base: usize,
    len: usize,
    typ: MemoryRegionType,
}

impl MemoryRegion {
    pub const fn new(base: Paddr, len: usize, typ: MemoryRegionType) -> Self {
        MemoryRegion { base, len, typ }
    }

    pub const fn bad() -> Self {
        MemoryRegion {
            base: 0,
            len: 0,
            typ: MemoryRegionType::BadMemory,
        }
    }

    pub fn kernel() -> Self {
        unsafe extern "C" {
            fn __kernel_start();
            fn __kernel_end();
        }
        MemoryRegion {
            base: __kernel_start as *const () as usize - kernel_loaded_offset(),
            len: __kernel_end as *const () as usize - __kernel_start as *const () as usize,
            typ: MemoryRegionType::Kernel,
        }
    }

    pub fn framebuffer(fb: &crate::boot::BootloaderFramebufferArg) -> Self {
        Self {
            base: fb.address,
            len: (fb.width * fb.height * fb.bpp).div_ceil(8),
            typ: MemoryRegionType::Framebuffer,
        }
    }

    pub fn module(bytes: &[u8]) -> Self {
        let vaddr = bytes.as_ptr() as Vaddr;
        assert!(crate::mm::kspace::LINEAR_MAPPING_VADDR_RANGE.contains(&vaddr));

        Self {
            base: vaddr - crate::mm::kspace::LINEAR_MAPPING_BASE_VADDR,
            len: bytes.len(),
            typ: MemoryRegionType::Reclaimable,
        }
    }

    pub fn base(&self) -> Paddr {
        self.base
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn end(&self) -> Paddr {
        self.base + self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn typ(&self) -> MemoryRegionType {
        self.typ
    }

    fn as_aligned(&self) -> Self {
        let (base, len) = match self.typ() {
            MemoryRegionType::Usable => {
                let aligned_base = self.base().align_up(PAGE_SIZE);
                let aligned_end = self.end().align_down(PAGE_SIZE);
                (aligned_base, aligned_end.saturating_sub(aligned_base))
            }
            _ => {
                let aligned_base = self.base().align_down(PAGE_SIZE);
                let aligned_end = self.end().align_up(PAGE_SIZE);
                (aligned_base, aligned_end - aligned_base)
            }
        };
        MemoryRegion {
            base,
            len,
            typ: self.typ,
        }
    }
}

const MAX_REGIONS: usize = 512;

pub(crate) struct MemoryRegionArray<const LEN: usize = MAX_REGIONS> {
    regions: [MemoryRegion; LEN],
    count: usize,
}

impl<const LEN: usize> Default for MemoryRegionArray<LEN> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const LEN: usize> Deref for MemoryRegionArray<LEN> {
    type Target = [MemoryRegion];

    fn deref(&self) -> &Self::Target {
        &self.regions[..self.count]
    }
}

#[derive(Debug)]
pub(crate) struct ArrayFullError;

impl<const LEN: usize> MemoryRegionArray<LEN> {
    pub(crate) const fn new() -> Self {
        Self {
            regions: [MemoryRegion::bad(); LEN],
            count: 0,
        }
    }

    pub(crate) fn push(&mut self, region: MemoryRegion) -> Result<(), ArrayFullError> {
        if self.count >= self.regions.len() {
            return Err(ArrayFullError);
        }

        self.regions[self.count] = region;
        self.count += 1;

        Ok(())
    }

    pub(crate) fn into_non_overlapping(mut self) -> Self {
        let mut count = 0;
        for i in 0..self.count {
            let region = self.regions[i].as_aligned();
            if !region.is_empty() {
                self.regions[count] = region;
                count += 1;
            }
        }
        self.count = count;

        let max_addr = self.iter().map(|r| r.end()).max().unwrap_or(0);

        let mut result = MemoryRegionArray::<LEN>::new();

        let mut cur_right = 0;

        while cur_right < max_addr {
            let typ = self
                .iter()
                .filter(|region| (region.base()..region.end()).contains(&cur_right))
                .map(|region| region.typ())
                .min()
                .unwrap_or(MemoryRegionType::Unknown);

            let right = self
                .iter()
                .filter_map(|region| {
                    if region.base() > cur_right {
                        Some(region.base())
                    } else if region.end() > cur_right {
                        Some(region.end())
                    } else {
                        None
                    }
                })
                .min()
                .unwrap();

            result
                .push(MemoryRegion::new(cur_right, right - cur_right, typ))
                .unwrap();

            cur_right = right;
        }

        if result.is_empty() {
            return result;
        }

        let mut merged_count = 1;
        for i in 1..result.count {
            if result[i].typ() == result.regions[merged_count - 1].typ() {
                result.regions[merged_count - 1] = MemoryRegion::new(
                    result.regions[merged_count - 1].base(),
                    result.regions[merged_count - 1].len() + result[i].len(),
                    result.regions[merged_count - 1].typ(),
                );
            } else {
                result.regions[merged_count] = result[i];
                merged_count += 1;
            }
        }
        result.count = merged_count;

        result
    }
}

#[cfg(ktest)]
mod test {
    use super::*;
    use crate::prelude::ktest;

    #[ktest]
    fn sort_full_non_overlapping() {
        let mut regions = MemoryRegionArray::<64>::new();
        regions
            .push(MemoryRegion::new(
                0,
                PAGE_SIZE + 1,
                MemoryRegionType::Usable,
            ))
            .unwrap();
        regions
            .push(MemoryRegion::new(
                PAGE_SIZE - 1,
                PAGE_SIZE + 2,
                MemoryRegionType::Usable,
            ))
            .unwrap();
        regions
            .push(MemoryRegion::new(
                PAGE_SIZE * 2,
                PAGE_SIZE * 5,
                MemoryRegionType::Usable,
            ))
            .unwrap();
        regions
            .push(MemoryRegion::new(
                PAGE_SIZE * 3 + 1,
                PAGE_SIZE - 2,
                MemoryRegionType::BadMemory,
            ))
            .unwrap();
        regions
            .push(MemoryRegion::new(
                PAGE_SIZE * 9,
                PAGE_SIZE * 2,
                MemoryRegionType::Usable,
            ))
            .unwrap();

        let regions = regions.into_non_overlapping();

        assert_eq!(regions.count, 5);
        assert_eq!(regions[0].base(), 0);
        assert_eq!(regions[0].len(), PAGE_SIZE * 3);
        assert_eq!(regions[0].typ(), MemoryRegionType::Usable);

        assert_eq!(regions[1].base(), PAGE_SIZE * 3);
        assert_eq!(regions[1].len(), PAGE_SIZE);
        assert_eq!(regions[1].typ(), MemoryRegionType::BadMemory);

        assert_eq!(regions[2].base(), PAGE_SIZE * 4);
        assert_eq!(regions[2].len(), PAGE_SIZE * 3);
        assert_eq!(regions[2].typ(), MemoryRegionType::Usable);

        assert_eq!(regions[3].base(), PAGE_SIZE * 7);
        assert_eq!(regions[3].len(), PAGE_SIZE * 2);
        assert_eq!(regions[3].typ(), MemoryRegionType::Unknown);

        assert_eq!(regions[4].base(), PAGE_SIZE * 9);
        assert_eq!(regions[4].len(), PAGE_SIZE * 2);
        assert_eq!(regions[4].typ(), MemoryRegionType::Usable);
    }

    #[ktest]
    fn into_non_overlapping_drops_usable_subpage_regions() {
        let mut regions = MemoryRegionArray::<64>::new();
        let bases = [100 * PAGE_SIZE, 100 * PAGE_SIZE + 123];
        let subpage_lens = [0, PAGE_SIZE / 2, PAGE_SIZE - 1];
        for base in bases {
            for len in subpage_lens {
                regions
                    .push(MemoryRegion::new(base, len, MemoryRegionType::Usable))
                    .unwrap();
            }
        }
        let regions = regions.into_non_overlapping();
        assert!(regions.is_empty());
    }
}
