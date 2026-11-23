use aster_util::{field_ptr, safe_ptr::SafePtr};
use ostd::mm::dma::DmaCoherent;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct NvmeCommand {
    opcode: u8,
    flags: u8,
    cid: u16,
    nsid: u32,
    _rsvd: u64,
    mptr: u64,
    dptr: [u64; 2],
    cdw10: u32,
    cdw11: u32,
    cdw12: u32,
    cdw13: u32,
    cdw14: u32,
    cdw15: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct NvmeCompletion {
    dword0: u32,

    dword1: u32,

    sq_head: u16,

    sq_id: u16,

    cid: u16,

    status: u16,
}

impl NvmeCommand {
    pub(crate) fn from_raw_fields<const N: usize>(
        opcode: u8,
        flags: u8,
        nsid: u32,
        dptr: [u64; 2],
        cdw: [u32; N],
    ) -> Self {
        const { assert!(N <= 6) };
        Self {
            opcode,
            flags,
            cid: 0,
            nsid,
            _rsvd: 0,
            mptr: 0,
            dptr,
            cdw10: cdw.first().copied().unwrap_or(0),
            cdw11: cdw.get(1).copied().unwrap_or(0),
            cdw12: cdw.get(2).copied().unwrap_or(0),
            cdw13: cdw.get(3).copied().unwrap_or(0),
            cdw14: cdw.get(4).copied().unwrap_or(0),
            cdw15: cdw.get(5).copied().unwrap_or(0),
        }
    }

    pub(crate) fn set_cid(&mut self, cid: u16) {
        self.cid = cid;
    }
}

impl NvmeCompletion {
    pub(crate) fn read_phase_tag(ring_slot_ptr: &SafePtr<NvmeCompletion, &DmaCoherent>) -> bool {
        let status = field_ptr!(ring_slot_ptr, NvmeCompletion, status)
            .read_once()
            .expect("CQ status field must be valid within allocated DMA ring");
        (status & 1) != 0
    }

    pub(crate) fn sq_head(&self) -> u16 {
        self.sq_head
    }

    pub(crate) fn sq_id(&self) -> u16 {
        self.sq_id
    }

    pub(crate) fn cid(&self) -> u16 {
        self.cid
    }

    pub(crate) fn status(&self) -> u16 {
        self.status
    }

    const STATUS_SC_MASK: u16 = 0x01FE;

    pub(crate) fn has_error(&self) -> bool {
        self.status_code() != 0
    }

    pub(crate) fn status_code(&self) -> u8 {
        ((self.status & Self::STATUS_SC_MASK) >> 1) as u8
    }
}
