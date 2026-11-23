use core::ops::Range;

use ostd::sync::RwMutexReadGuard;

use super::{VmMapping, Vmar, VmarInner};

impl Vmar {
    pub(crate) fn query(&self, range: Range<usize>) -> VmarQueryGuard<'_> {
        VmarQueryGuard {
            vmar: self.inner.read(),
            range,
        }
    }
}

pub(crate) struct VmarQueryGuard<'a> {
    vmar: RwMutexReadGuard<'a, VmarInner>,
    range: Range<usize>,
}

impl VmarQueryGuard<'_> {
    pub(crate) fn iter(&self) -> impl Iterator<Item = &VmMapping> {
        self.vmar.query(&self.range)
    }

    pub(crate) fn is_fully_mapped(&self) -> bool {
        let mut last_mapping_end = self.range.start;

        for mapping in self.iter() {
            if last_mapping_end < mapping.map_to_addr() {
                return false;
            }
            last_mapping_end = mapping.map_end();
        }

        last_mapping_end >= self.range.end
    }
}
