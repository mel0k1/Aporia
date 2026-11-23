mod cell;
mod dyn_cpu_local;
mod static_cpu_local;

pub(crate) mod single_instr;

use core::{alloc::Layout, marker::PhantomData, ops::Deref};

use align_ext::AlignExt;
pub use cell::CpuLocalCell;
pub(crate) use cell::CpuLocalPtr;
pub use dyn_cpu_local::DynCpuLocalChunk;
use dyn_cpu_local::DynamicStorage;
use spin::Once;
use static_cpu_local::StaticStorage;

use super::CpuId;
use crate::{
    irq::DisabledLocalIrqGuard,
    mm::{PAGE_SIZE, Paddr, frame::allocator, paddr_to_vaddr},
    util::id_set::Id,
};

pub type DynamicCpuLocal<T> = CpuLocal<T, DynamicStorage<T>>;

pub type StaticCpuLocal<T> = CpuLocal<T, StaticStorage<T>>;

unsafe extern "C" {
    fn __cpu_local_start();
    fn __cpu_local_end();
}

pub unsafe trait AnyStorage<T> {
    fn get_ptr_on_current(&self, guard: &DisabledLocalIrqGuard) -> *const T;

    fn get_ptr_on_target(&self, cpu: CpuId) -> *const T;
}

pub struct CpuLocal<T, S: AnyStorage<T>> {
    storage: S,
    phantom: PhantomData<T>,
}

impl<T: 'static, S: AnyStorage<T>> CpuLocal<T, S> {
    pub fn get_with<'a>(
        &'a self,
        guard: &'a DisabledLocalIrqGuard,
    ) -> CpuLocalDerefGuard<'a, T, S> {
        CpuLocalDerefGuard {
            cpu_local: self,
            guard,
        }
    }
}

impl<T: 'static + Sync, S: AnyStorage<T>> CpuLocal<T, S> {
    pub fn get_on_cpu(&self, target_cpu_id: CpuId) -> &T {
        let ptr = self.storage.get_ptr_on_target(target_cpu_id);
        unsafe { &*ptr }
    }
}

#[must_use]
pub struct CpuLocalDerefGuard<'a, T: 'static, S: AnyStorage<T>> {
    cpu_local: &'a CpuLocal<T, S>,
    guard: &'a DisabledLocalIrqGuard,
}

impl<'a, T: 'static, S: AnyStorage<T>> CpuLocalDerefGuard<'a, T, S> {
    pub fn deref_target(&self) -> &'a T {
        is_used::debug_set_true();

        let ptr = self.cpu_local.storage.get_ptr_on_current(self.guard);
        unsafe { &*ptr }
    }
}

impl<'a, T: 'static, S: AnyStorage<T>> Deref for CpuLocalDerefGuard<'a, T, S> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.deref_target()
    }
}

unsafe impl<T: Send + 'static, S: AnyStorage<T>> Sync for CpuLocal<T, S> {}
unsafe impl<T: Send + 'static> Send for CpuLocal<T, DynamicStorage<T>> {}

impl<T: 'static, S: AnyStorage<T>> !Copy for CpuLocal<T, S> {}
impl<T: 'static, S: AnyStorage<T>> !Clone for CpuLocal<T, S> {}

static CPU_LOCAL_STORAGES: Once<&'static [Paddr]> = Once::new();

pub(crate) unsafe fn copy_bsp_for_ap(num_cpus: usize) {
    let num_aps = num_cpus - 1;
    if num_aps == 0 {
        return;
    }

    let res = {
        let size = size_of::<Paddr>()
            .checked_mul(num_aps)
            .unwrap()
            .align_up(PAGE_SIZE);
        let addr =
            allocator::early_alloc(Layout::from_size_align(size, PAGE_SIZE).unwrap()).unwrap();
        let ptr = paddr_to_vaddr(addr) as *mut Paddr;

        unsafe {
            core::ptr::write_bytes(ptr as *mut u8, 0, size);
        }
        unsafe { core::slice::from_raw_parts_mut(ptr, num_aps) }
    };

    let bsp_base_va = __cpu_local_start as *const () as usize;
    let bsp_end_va = __cpu_local_end as *const () as usize;

    for res_addr_mut in res.iter_mut() {
        let nbytes = (bsp_end_va - bsp_base_va).align_up(PAGE_SIZE);
        let ap_pages =
            allocator::early_alloc(Layout::from_size_align(nbytes, PAGE_SIZE).unwrap()).unwrap();
        let ap_pages_ptr = paddr_to_vaddr(ap_pages) as *mut u8;

        unsafe {
            core::ptr::copy_nonoverlapping(bsp_base_va as *const u8, ap_pages_ptr, nbytes);
        }

        *res_addr_mut = ap_pages;
    }

    is_used::debug_assert_false();

    assert!(!CPU_LOCAL_STORAGES.is_completed());
    CPU_LOCAL_STORAGES.call_once(|| res);
}

pub(crate) fn get_ap(cpu_id: CpuId) -> Paddr {
    let offset = cpu_id
        .as_usize()
        .checked_sub(1)
        .expect("The BSP does not have allocated CPU-local storage");

    let paddr = CPU_LOCAL_STORAGES
        .get()
        .expect("No CPU-local storage has been allocated")[offset];
    assert_ne!(
        paddr,
        0,
        "The CPU-local storage for CPU {} is not allocated",
        cpu_id.as_usize(),
    );
    paddr
}

mod is_used {

    cfg_select! {
        debug_assertions => {
            use core::sync::atomic::{AtomicBool, Ordering};

            static IS_USED: AtomicBool = AtomicBool::new(false);

            pub(super) fn debug_set_true() {
                IS_USED.store(true, Ordering::Relaxed);
            }

            pub(super) fn debug_assert_false() {
                debug_assert!(!IS_USED.load(Ordering::Relaxed));
            }
        }
        _ => {
            pub(super) fn debug_set_true() {}

            pub(super) fn debug_assert_false() {}
        }
    }
}

#[cfg(ktest)]
mod test {
    use core::cell::RefCell;

    use ostd_macros::ktest;

    #[ktest]
    fn cpu_local() {
        crate::cpu_local! {
            static FOO: RefCell<usize> = RefCell::new(1);
        }
        let irq_guard = crate::irq::disable_local();
        let foo_guard = FOO.get_with(&irq_guard);
        assert_eq!(*foo_guard.borrow(), 1);
        *foo_guard.borrow_mut() = 2;
        assert_eq!(*foo_guard.borrow(), 2);
        drop(foo_guard);
    }

    #[ktest]
    fn cpu_local_cell() {
        crate::cpu_local_cell! {
            static BAR: usize = 3;
        }
        let _guard = crate::irq::disable_local();
        assert_eq!(BAR.load(), 3);
        BAR.store(4);
        assert_eq!(BAR.load(), 4);
    }
}
