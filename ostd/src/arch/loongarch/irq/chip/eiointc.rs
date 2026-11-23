use loongArch64::iocsr::{iocsr_read_d, iocsr_write_b, iocsr_write_d, iocsr_write_h};

const OTHER_FUNCTION_SETTING_REG: usize = 0x420;

pub(super) struct Eiointc;

impl Eiointc {
    const U64_BITS: usize = u64::BITS as _;
    const U64_BYTES: usize = u64::BITS as usize / 8;
    const MAX_CORE_NUM: usize = 4;
    const MAX_INTERRUPT_NUM: usize = 256;

    const EXT_IOI_EN_BASE: usize = 0x1600;
    const EXT_IOI_BOUNCE_BASE: usize = 0x1680;
    const EXT_IOI_SR_BASE: usize = 0x1700;
    const PER_CORE_EXT_IOI_SR_BASE: usize = 0x1800;
    const EXT_IOI_MAP_BASE: usize = 0x14C0;
    const MAX_EXT_IOI_MAP_NUM: usize = 8;
    const EXT_IOI_MAP_CORE_BASE: usize = 0x1C00;
    const EXT_IOI_NODE_TYPE_BASE: usize = 0x14A0;

    pub(super) fn init(core_num: usize) {
        assert!(core_num <= Self::MAX_CORE_NUM);

        let mut v = iocsr_read_d(OTHER_FUNCTION_SETTING_REG);
        v |= 1 << 48;
        iocsr_write_d(OTHER_FUNCTION_SETTING_REG, v);

        let mut v = iocsr_read_d(OTHER_FUNCTION_SETTING_REG);
        v |= 1 << 49;
        iocsr_write_d(OTHER_FUNCTION_SETTING_REG, v);

        for i in 0..Self::MAX_EXT_IOI_MAP_NUM {
            iocsr_write_b(Self::EXT_IOI_MAP_BASE + i, i as _);
        }

        for i in 0..Self::MAX_INTERRUPT_NUM {
            iocsr_write_b(Self::EXT_IOI_MAP_CORE_BASE + i, (1 << core_num) - 1);
        }

        iocsr_write_h(Self::EXT_IOI_NODE_TYPE_BASE, 0x01);
    }

    pub(super) fn enable(irq: u8) {
        let reg_no = irq as usize / Self::U64_BITS;
        let bit_offset = irq as usize % Self::U64_BITS;

        let mut enable = iocsr_read_d(Self::EXT_IOI_EN_BASE + reg_no * Self::U64_BYTES);
        enable |= 1u64 << bit_offset;
        iocsr_write_d(Self::EXT_IOI_EN_BASE + reg_no * Self::U64_BYTES, enable);

        let mut bounce = iocsr_read_d(Self::EXT_IOI_BOUNCE_BASE + reg_no * Self::U64_BYTES);
        bounce |= 1u64 << bit_offset;
        iocsr_write_d(Self::EXT_IOI_BOUNCE_BASE + reg_no * Self::U64_BYTES, bounce);
    }

    pub(super) fn disable(irq: u8) {
        let reg_no = irq as usize / Self::U64_BITS;
        let bit_offset = irq as usize % Self::U64_BITS;

        let mut enable = iocsr_read_d(Self::EXT_IOI_EN_BASE + reg_no * Self::U64_BYTES);
        enable &= !(1u64 << bit_offset);
        iocsr_write_d(Self::EXT_IOI_EN_BASE + reg_no * Self::U64_BYTES, enable);

        let mut bounce = iocsr_read_d(Self::EXT_IOI_BOUNCE_BASE + reg_no * Self::U64_BYTES);
        bounce &= !(1u64 << bit_offset);
        iocsr_write_d(Self::EXT_IOI_BOUNCE_BASE + reg_no * Self::U64_BYTES, bounce);
    }

    pub(super) fn claim() -> Option<u8> {
        for i in 0..Self::MAX_INTERRUPT_NUM / Self::U64_BITS {
            let status = iocsr_read_d(Self::PER_CORE_EXT_IOI_SR_BASE + i * Self::U64_BYTES);
            if status != 0 {
                let irq = (i * Self::U64_BITS + Self::U64_BITS
                    - 1
                    - status.leading_zeros() as usize) as _;
                return Some(irq);
            }
        }
        None
    }

    pub(super) fn complete(irq: u8) {
        let reg_no = irq as usize / Self::U64_BITS;
        let bit_offset = irq as usize % Self::U64_BITS;

        let status = 1u64 << bit_offset;
        iocsr_write_d(
            Self::PER_CORE_EXT_IOI_SR_BASE + reg_no * Self::U64_BYTES,
            status,
        );
    }
}
