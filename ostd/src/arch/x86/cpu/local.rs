use x86_64::registers::segmentation::{GS, Segment64};

pub(crate) fn get_base() -> u64 {
    GS::read_base().as_u64()
}

use crate::cpu::local::single_instr::{
    SingleInstructionAddAssign, SingleInstructionBitAndAssign, SingleInstructionBitOrAssign,
    SingleInstructionBitXorAssign, SingleInstructionLoad, SingleInstructionStore,
    SingleInstructionSubAssign,
};

macro_rules! impl_numeric_single_instruction_for {
    ($([$typ: ty, $inout_type: ident, $register_format: expr])*) => {$(

        impl SingleInstructionAddAssign<$typ> for $typ {
            unsafe fn add_assign(offset: usize, val: Self) {
                unsafe {
                    core::arch::asm!(
                        concat!("add gs:[{0}], {1", $register_format, "}"),
                        in(reg) offset,
                        in($inout_type) val,
                        options(nostack),
                    );
                }
            }
        }

        impl SingleInstructionSubAssign<$typ> for $typ {
            unsafe fn sub_assign(offset: usize, val: Self) {
                unsafe {
                    core::arch::asm!(
                        concat!("sub gs:[{0}], {1", $register_format, "}"),
                        in(reg) offset,
                        in($inout_type) val,
                        options(nostack),
                    );
                }
            }
        }

        impl SingleInstructionBitAndAssign<$typ> for $typ {
            unsafe fn bitand_assign(offset: usize, val: Self) {
                unsafe {
                    core::arch::asm!(
                        concat!("and gs:[{0}], {1", $register_format, "}"),
                        in(reg) offset,
                        in($inout_type) val,
                        options(nostack),
                    );
                }
            }
        }

        impl SingleInstructionBitOrAssign<$typ> for $typ {
            unsafe fn bitor_assign(offset: usize, val: Self) {
                unsafe {
                    core::arch::asm!(
                        concat!("or gs:[{0}], {1", $register_format, "}"),
                        in(reg) offset,
                        in($inout_type) val,
                        options(nostack),
                    );
                }
            }
        }

        impl SingleInstructionBitXorAssign<$typ> for $typ {
            unsafe fn bitxor_assign(offset: usize, val: Self) {
                unsafe {
                    core::arch::asm!(
                        concat!("xor gs:[{0}], {1", $register_format, "}"),
                        in(reg) offset,
                        in($inout_type) val,
                        options(nostack),
                    );
                }
            }
        }

        impl SingleInstructionLoad for $typ {
            unsafe fn load(offset: usize) -> Self {
                let val: Self;
                unsafe {
                    core::arch::asm!(
                        concat!("mov {0", $register_format, "}, gs:[{1}]"),
                        out($inout_type) val,
                        in(reg) offset,
                        options(nostack, readonly),
                    );
                }
                val
            }
        }

        impl SingleInstructionStore for $typ {
            unsafe fn store(offset: usize, val: Self) {
                unsafe {
                    core::arch::asm!(
                        concat!("mov gs:[{0}], {1", $register_format, "}"),
                        in(reg) offset,
                        in($inout_type) val,
                        options(nostack),
                    );
                }
            }
        }

    )*};
}

impl_numeric_single_instruction_for!(
    [u64,   reg,    ":r"]
    [usize, reg,    ":r"]
    [u32,   reg,    ":e"]
    [u16,   reg,    ":x"]
    [u8,    reg_byte, ""]
    [i64,   reg,    ":r"]
    [isize, reg,    ":r"]
    [i32,   reg,    ":e"]
    [i16,   reg,    ":x"]
    [i8,    reg_byte, ""]
);

macro_rules! impl_generic_single_instruction_for {
    ($([<$gen_type:ident $(, $more_gen_type:ident)*>, $typ:ty])*) => {$(

        impl<$gen_type $(, $more_gen_type)*> SingleInstructionLoad for $typ {
            unsafe fn load(offset: usize) -> Self {
                let val: Self;
                unsafe {
                    core::arch::asm!(
                        concat!("mov {0}, gs:[{1}]"),
                        out(reg) val,
                        in(reg) offset,
                        options(nostack, readonly),
                    );
                }
                val
            }
        }

        impl<$gen_type $(, $more_gen_type)*> SingleInstructionStore for $typ {
            unsafe fn store(offset: usize, val: Self) {
                unsafe {
                    core::arch::asm!(
                        concat!("mov gs:[{0}], {1}"),
                        in(reg) offset,
                        in(reg) val,
                        options(nostack),
                    );
                }
            }
        }
    )*}
}

impl_generic_single_instruction_for!(
    [<T>, *const T]
    [<T>, *mut T]
    [<T, R>, fn(T) -> R]
);

impl SingleInstructionLoad for bool {
    unsafe fn load(offset: usize) -> Self {
        let val: u8;
        unsafe {
            core::arch::asm!(
                "mov {0}, gs:[{1}]",
                out(reg_byte) val,
                in(reg) offset,
                options(nostack, readonly),
            );
        }
        debug_assert!(val == 1 || val == 0);
        val == 1
    }
}

impl SingleInstructionStore for bool {
    unsafe fn store(offset: usize, val: Self) {
        let val: u8 = if val { 1 } else { 0 };
        unsafe {
            core::arch::asm!(
                "mov gs:[{0}], {1}",
                in(reg) offset,
                in(reg_byte) val,
                options(nostack),
            );
        }
    }
}
