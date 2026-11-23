pub use current::PinCurrentCpu;
pub use set::{AtomicCpuSet, CpuSet};

use crate::util::id_set::Id;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CpuId(u32);

impl CpuId {
    pub fn new(raw_id: u32) -> Self {
        assert!(raw_id < num_cpus() as u32);
        unsafe { Self::new_unchecked(raw_id) }
    }

    pub const fn bsp() -> Self {
        let bsp_raw_cpu_id = 0;
        Self(bsp_raw_cpu_id)
    }
}

impl From<CpuId> for u32 {
    fn from(cpu_id: CpuId) -> Self {
        cpu_id.0
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CpuIdFromIntError;

impl TryFrom<usize> for CpuId {
    type Error = CpuIdFromIntError;

    fn try_from(raw_id: usize) -> Result<Self, Self::Error> {
        if raw_id < num_cpus() {
            let new_self = unsafe { CpuId::new_unchecked(raw_id as u32) };
            Ok(new_self)
        } else {
            Err(CpuIdFromIntError)
        }
    }
}

pub fn num_cpus() -> usize {
    (unsafe { NUM_CPUS }) as usize
}

static mut NUM_CPUS: u32 = 1;

pub fn all_cpus() -> impl Iterator<Item = CpuId> {
    (0..num_cpus()).map(|raw_id| unsafe { CpuId::new_unchecked(raw_id as u32) })
}

mod set {
    use super::{CpuId, num_cpus};
    use crate::util::id_set::{AtomicIdSet, Id, IdSet};

    pub type CpuSet = IdSet<CpuId>;

    pub type AtomicCpuSet = AtomicIdSet<CpuId>;

    unsafe impl Id for CpuId {
        unsafe fn new_unchecked(raw_id: u32) -> Self {
            Self(raw_id)
        }

        fn cardinality() -> u32 {
            num_cpus() as u32
        }
    }
}

mod current {

    use super::CpuId;
    use crate::{cpu_local_cell, task::atomic_mode::InAtomicMode, util::id_set::Id};

    pub unsafe trait PinCurrentCpu {
        fn current_cpu(&self) -> CpuId {
            CpuId::current_racy()
        }
    }

    unsafe impl<T: InAtomicMode> PinCurrentCpu for T {}
    unsafe impl PinCurrentCpu for dyn InAtomicMode + '_ {}

    impl CpuId {
        pub fn current_racy() -> Self {
            #[cfg(debug_assertions)]
            assert!(IS_CURRENT_CPU_INITED.load());

            let current_raw_id = CURRENT_CPU.load();
            unsafe { Self::new_unchecked(current_raw_id) }
        }
    }

    pub(super) unsafe fn init_on_cpu(current_cpu_id: u32) {
        CURRENT_CPU.store(current_cpu_id);

        #[cfg(debug_assertions)]
        IS_CURRENT_CPU_INITED.store(true);
    }

    cpu_local_cell! {
        pub(super) static CURRENT_CPU: u32 = 0;
        #[cfg(debug_assertions)]
        pub(super) static IS_CURRENT_CPU_INITED: bool = false;
    }
}

pub(super) unsafe fn init_on_bsp(num_cpus: u32) {
    unsafe { current::init_on_cpu(0) };

    unsafe { init_num_cpus(num_cpus) };
}

unsafe fn init_num_cpus(num_cpus: u32) {
    assert!(num_cpus >= 1);

    unsafe { NUM_CPUS = num_cpus };
}

pub(super) unsafe fn init_on_ap(cpu_id: u32) {
    unsafe { current::init_on_cpu(cpu_id) };
}
