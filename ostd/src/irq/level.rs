use crate::{cpu::PrivilegeLevel, cpu_local_cell};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InterruptLevel {
    L0,
    L1(PrivilegeLevel),
    L2,
}

impl !Send for InterruptLevel {}
impl !Sync for InterruptLevel {}

impl InterruptLevel {
    pub fn current() -> Self {
        const LEVEL_VAL_OFFSET: u8 = 1;
        const CPU_PRIV_MASK: u8 = 1 << 0;

        let raw_level = INTERRUPT_LEVEL.load();
        let level = raw_level >> LEVEL_VAL_OFFSET;
        match level {
            0 => Self::L0,
            1 => {
                let cpu_priv_at_irq = if (raw_level & CPU_PRIV_MASK) == 0 {
                    PrivilegeLevel::Kernel
                } else {
                    PrivilegeLevel::User
                };
                Self::L1(cpu_priv_at_irq)
            }
            2 => Self::L2,
            _ => unreachable!("level must between 0 and 2 (inclusive)"),
        }
    }

    pub fn as_u8(&self) -> u8 {
        match self {
            Self::L0 => 0,
            Self::L1(_) => 1,
            Self::L2 => 2,
        }
    }

    pub fn is_task_context(&self) -> bool {
        *self == Self::L0
    }

    pub fn is_interrupt_context(&self) -> bool {
        matches!(self, Self::L1(_) | Self::L2)
    }
}

pub(super) fn enter<F: FnOnce()>(f: F, cpu_priv_at_irq: PrivilegeLevel) {
    let increment = {
        let bit_0 = match cpu_priv_at_irq {
            PrivilegeLevel::Kernel => 0,
            PrivilegeLevel::User => 1,
        };
        let bit_1 = 0b10;
        bit_1 | bit_0
    };
    INTERRUPT_LEVEL.add_assign(increment);

    f();

    INTERRUPT_LEVEL.sub_assign(increment);
}

cpu_local_cell! {
    static INTERRUPT_LEVEL: u8 = 0;
}
