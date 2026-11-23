#[repr(usize)]
#[derive(Clone, Copy, Debug)]
#[expect(dead_code)]
pub(crate) enum NvmeRegs32 {
    Vs = 0x8,
    Intms = 0xC,
    Intmc = 0x10,
    Cc = 0x14,
    Csts = 0x1C,
    Nssr = 0x20,
    Aqa = 0x24,
    Cmbloc = 0x38,
    Cmbsz = 0x3C,
    Bpinfo = 0x40,
    Bprsel = 0x44,
    Bpmbl = 0x48,
    Cmbsts = 0x58,
    Pmrcap = 0xE00,
    Pmrctl = 0xE04,
    Pmrsts = 0xE08,
    Pmrebs = 0xE0C,
    Pmrswtp = 0xE10,
}

#[repr(usize)]
#[derive(Clone, Copy, Debug)]
#[expect(dead_code)]
pub(crate) enum NvmeRegs64 {
    Cap = 0x0,
    Asq = 0x28,
    Acq = 0x30,
    Cmbmsc = 0x50,
    Pmrmsc = 0xE14,
}

pub(crate) const NVME_BAR0_FIXED_REGS_END: u64 = NvmeRegs64::Pmrmsc as u64 + 8;

impl NvmeRegs64 {
    pub(crate) const CAP_DSTRD_SHIFT: u32 = 32;
    pub(crate) const CAP_DSTRD_MASK: u64 = 0b1111;
    pub(crate) const CAP_TO_SHIFT: u32 = 24;
    pub(crate) const CAP_TO_MASK: u64 = 0xff;
    pub(crate) const CAP_MPSMIN_SHIFT: u32 = 48;
    pub(crate) const CAP_MPSMIN_MASK: u64 = 0b1111;
    pub(crate) const CAP_MPSMAX_SHIFT: u32 = 52;
    pub(crate) const CAP_MPSMAX_MASK: u64 = 0b1111;
}

#[repr(usize)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum NvmeDoorbellRegs {
    Sqtdbl,
    Cqhdbl,
}

impl NvmeDoorbellRegs {
    pub(crate) fn offset(&self, qid: u16, dstrd: u16) -> usize {
        const DOORBELL_BASE: usize = 0x1000;
        let stride = 4usize << usize::from(dstrd);

        match self {
            NvmeDoorbellRegs::Sqtdbl => DOORBELL_BASE + (2 * usize::from(qid)) * stride,
            NvmeDoorbellRegs::Cqhdbl => DOORBELL_BASE + ((2 * usize::from(qid)) + 1) * stride,
        }
    }
}
