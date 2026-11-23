use core::{cell::UnsafeCell, fmt};

use super::{__cpu_local_end, __cpu_local_start, single_instr::*};
use crate::{arch, mm::PAGE_SIZE};

#[macro_export]
macro_rules! cpu_local_cell {
    ($( $(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr; )*) => {
        $(
            #[unsafe(link_section = ".cpu_local")]
            $(#[$attr])* $vis static $name: $crate::cpu::local::CpuLocalCell<$t> = {
                let val = $init;
                unsafe {
                    $crate::cpu::local::CpuLocalCell::__new(val)
                }
            };
        )*
    };
}

pub struct CpuLocalCell<T: 'static>(UnsafeCell<T>);

impl<T: 'static> CpuLocalCell<T> {
    #[doc(hidden)]
    pub const unsafe fn __new(val: T) -> Self {
        const { assert!(align_of::<T>() <= PAGE_SIZE) };
        Self(UnsafeCell::new(val))
    }

    pub fn as_mut_ptr(&'static self) -> *mut T {
        super::is_used::debug_set_true();

        let offset = {
            let bsp_va = self as *const _ as usize;
            let bsp_base = __cpu_local_start as *const () as usize;
            debug_assert!(bsp_va + size_of::<T>() <= __cpu_local_end as *const () as usize);

            bsp_va - bsp_base
        };

        let local_base = arch::cpu::local::get_base() as usize;
        let local_va = local_base + offset;

        debug_assert_eq!(local_va % align_of::<T>(), 0);

        local_va as *mut T
    }
}

unsafe impl<T: Send + 'static> Sync for CpuLocalCell<T> {}

impl<T: 'static> !Copy for CpuLocalCell<T> {}
impl<T: 'static> !Clone for CpuLocalCell<T> {}

impl<T: 'static> !Send for CpuLocalCell<T> {}

impl<T: 'static + SingleInstructionAddAssign<T>> CpuLocalCell<T> {
    pub fn add_assign(&'static self, rhs: T) {
        let offset = self as *const _ as usize - __cpu_local_start as *const () as usize;
        unsafe {
            T::add_assign(offset, rhs);
        }
    }
}

impl<T: 'static + SingleInstructionSubAssign<T>> CpuLocalCell<T> {
    pub fn sub_assign(&'static self, rhs: T) {
        let offset = self as *const _ as usize - __cpu_local_start as *const () as usize;
        unsafe {
            T::sub_assign(offset, rhs);
        }
    }
}

impl<T: 'static + SingleInstructionBitAndAssign<T>> CpuLocalCell<T> {
    pub fn bitand_assign(&'static self, rhs: T) {
        let offset = self as *const _ as usize - __cpu_local_start as *const () as usize;
        unsafe {
            T::bitand_assign(offset, rhs);
        }
    }
}

impl<T: 'static + SingleInstructionBitOrAssign<T>> CpuLocalCell<T> {
    pub fn bitor_assign(&'static self, rhs: T) {
        let offset = self as *const _ as usize - __cpu_local_start as *const () as usize;
        unsafe {
            T::bitor_assign(offset, rhs);
        }
    }
}

impl<T: 'static + SingleInstructionBitXorAssign<T>> CpuLocalCell<T> {
    pub fn bitxor_assign(&'static self, rhs: T) {
        let offset = self as *const _ as usize - __cpu_local_start as *const () as usize;
        unsafe {
            T::bitxor_assign(offset, rhs);
        }
    }
}

impl<T: 'static + SingleInstructionLoad> CpuLocalCell<T> {
    pub fn load(&'static self) -> T {
        let offset = self as *const _ as usize - __cpu_local_start as *const () as usize;
        unsafe { T::load(offset) }
    }
}

impl<T: 'static + SingleInstructionStore> CpuLocalCell<T> {
    pub fn store(&'static self, val: T) {
        let offset = self as *const _ as usize - __cpu_local_start as *const () as usize;
        unsafe {
            T::store(offset, val);
        }
    }
}

#[repr(transparent)]
pub(crate) struct CpuLocalPtr<T>(*const T);

impl<T> CpuLocalPtr<T> {
    pub(crate) const fn new(ptr: *const T) -> Self {
        Self(ptr)
    }

    pub(crate) const fn null() -> Self {
        Self(core::ptr::null())
    }

    pub(crate) const fn as_ptr(self) -> *const T {
        self.0
    }
}

impl<T> Clone for CpuLocalPtr<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for CpuLocalPtr<T> {}

impl<T> fmt::Debug for CpuLocalPtr<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("CpuLocalPtr").field(&self.0).finish()
    }
}

unsafe impl<T> Send for CpuLocalPtr<T> {}

impl<T> SingleInstructionLoad for CpuLocalPtr<T> {
    unsafe fn load(offset: usize) -> Self {
        Self(unsafe { <*const T as SingleInstructionLoad>::load(offset) })
    }
}

impl<T> SingleInstructionStore for CpuLocalPtr<T> {
    unsafe fn store(offset: usize, val: Self) {
        unsafe { <*const T as SingleInstructionStore>::store(offset, val.0) };
    }
}
