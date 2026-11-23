#![cfg_attr(
    any(
        target_arch = "riscv64",
        target_arch = "loongarch64",
        target_arch = "aarch64"
    ),
    expect(dead_code)
)]

use core::marker::PhantomData;

use super::{__cpu_local_end, __cpu_local_start, AnyStorage, CpuLocal};
use crate::{arch, cpu::CpuId, irq::DisabledLocalIrqGuard, mm::PAGE_SIZE, util::id_set::Id};

#[macro_export]
macro_rules! cpu_local {
    ($( $(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr; )*) => {
        $(
            #[unsafe(link_section = ".cpu_local")]
            $(#[$attr])* $vis static $name: $crate::cpu::local::StaticCpuLocal<$t> = {
                let val = $init;
                unsafe {
                    $crate::cpu::local::CpuLocal::__new_static(val)
                }
            };
        )*
    };
}

pub struct StaticStorage<T: 'static>(T);

impl<T: 'static> StaticStorage<T> {
    fn as_ptr(&self) -> *const T {
        super::is_used::debug_set_true();

        let offset = self.get_offset();

        let local_base = arch::cpu::local::get_base() as usize;
        let local_va = local_base + offset;

        debug_assert_eq!(local_va % align_of::<T>(), 0);

        local_va as *const T
    }

    fn get_offset(&self) -> usize {
        let bsp_va = self as *const _ as usize;
        let bsp_base = __cpu_local_start as *const () as usize;
        debug_assert!(bsp_va + size_of::<T>() <= __cpu_local_end as *const () as usize);

        bsp_va - bsp_base
    }
}

unsafe impl<T: 'static> AnyStorage<T> for StaticStorage<T> {
    fn get_ptr_on_current(&self, _guard: &DisabledLocalIrqGuard) -> *const T {
        self.as_ptr()
    }

    fn get_ptr_on_target(&self, cpu_id: CpuId) -> *const T {
        super::is_used::debug_set_true();

        let cpu_id = cpu_id.as_usize();

        if cpu_id == 0 {
            return &self.0 as *const T;
        }

        let base = {
            let storages = unsafe { super::CPU_LOCAL_STORAGES.get_unchecked() };
            let storage = unsafe { *storages.get_unchecked(cpu_id - 1) };
            crate::mm::paddr_to_vaddr(storage)
        };

        let offset = self.get_offset();
        (base + offset) as *const T
    }
}

impl<T: 'static> CpuLocal<T, StaticStorage<T>> {
    #[doc(hidden)]
    pub const unsafe fn __new_static(val: T) -> Self {
        const { assert!(align_of::<T>() <= PAGE_SIZE) };
        Self {
            storage: StaticStorage(val),
            phantom: PhantomData,
        }
    }

    pub(crate) fn as_ptr(&self) -> *const T {
        self.storage.as_ptr()
    }
}
