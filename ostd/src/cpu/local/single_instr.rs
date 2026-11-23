pub trait SingleInstructionAddAssign<Rhs = Self> {
    unsafe fn add_assign(offset: usize, rhs: Rhs);
}

impl<T: num_traits::WrappingAdd + Copy> SingleInstructionAddAssign<T> for T {
    default unsafe fn add_assign(offset: usize, rhs: T) {
        let _guard = crate::irq::disable_local();
        let base = crate::arch::cpu::local::get_base() as usize;
        let addr = (base + offset) as *mut Self;
        unsafe { addr.write(addr.read().wrapping_add(&rhs)) };
    }
}

pub trait SingleInstructionSubAssign<Rhs = Self> {
    unsafe fn sub_assign(offset: usize, rhs: Rhs);
}

impl<T: num_traits::WrappingSub + Copy> SingleInstructionSubAssign<T> for T {
    default unsafe fn sub_assign(offset: usize, rhs: T) {
        let _guard = crate::irq::disable_local();
        let base = crate::arch::cpu::local::get_base() as usize;
        let addr = (base + offset) as *mut Self;
        unsafe { addr.write(addr.read().wrapping_sub(&rhs)) };
    }
}

pub trait SingleInstructionBitOrAssign<Rhs = Self> {
    unsafe fn bitor_assign(offset: usize, rhs: Rhs);
}

impl<T: core::ops::BitOr<Output = T> + Copy> SingleInstructionBitOrAssign<T> for T {
    default unsafe fn bitor_assign(offset: usize, rhs: T) {
        let _guard = crate::irq::disable_local();
        let base = crate::arch::cpu::local::get_base() as usize;
        let addr = (base + offset) as *mut Self;
        unsafe { addr.write(addr.read() | rhs) };
    }
}

pub trait SingleInstructionBitAndAssign<Rhs = Self> {
    unsafe fn bitand_assign(offset: usize, rhs: Rhs);
}

impl<T: core::ops::BitAnd<Output = T> + Copy> SingleInstructionBitAndAssign<T> for T {
    default unsafe fn bitand_assign(offset: usize, rhs: T) {
        let _guard = crate::irq::disable_local();
        let base = crate::arch::cpu::local::get_base() as usize;
        let addr = (base + offset) as *mut Self;
        unsafe { addr.write(addr.read() & rhs) };
    }
}

pub trait SingleInstructionBitXorAssign<Rhs = Self> {
    unsafe fn bitxor_assign(offset: usize, rhs: Rhs);
}

impl<T: core::ops::BitXor<Output = T> + Copy> SingleInstructionBitXorAssign<T> for T {
    default unsafe fn bitxor_assign(offset: usize, rhs: T) {
        let _guard = crate::irq::disable_local();
        let base = crate::arch::cpu::local::get_base() as usize;
        let addr = (base + offset) as *mut Self;
        unsafe { addr.write(addr.read() ^ rhs) };
    }
}

pub trait SingleInstructionLoad {
    unsafe fn load(offset: usize) -> Self;
}

impl<T: Copy> SingleInstructionLoad for T {
    default unsafe fn load(offset: usize) -> Self {
        let _guard = crate::irq::disable_local();
        let base = crate::arch::cpu::local::get_base() as usize;
        let ptr = (base + offset) as *const Self;
        unsafe { ptr.read() }
    }
}

pub trait SingleInstructionStore {
    unsafe fn store(offset: usize, val: Self);
}

impl<T: Copy> SingleInstructionStore for T {
    default unsafe fn store(offset: usize, val: Self) {
        let _guard = crate::irq::disable_local();
        let base = crate::arch::cpu::local::get_base() as usize;
        let ptr = (base + offset) as *mut Self;
        unsafe { ptr.write(val) };
    }
}
