use bitflags::bitflags;
use int_to_c_enum::TryFromInt;

#[repr(u16)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub enum VirtioVsockType {
    Stream = 1,
}

#[repr(u16)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub enum VirtioVsockOp {
    Request = 1,
    Response = 2,
    Rst = 3,
    Shutdown = 4,
    Rw = 5,
    CreditUpdate = 6,
    CreditRequest = 7,
}

bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub struct VirtioVsockShutdownFlags: u32 {
        const RECEIVE = 1;
        const SEND    = 2;
    }
}

#[repr(C, packed)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct VirtioVsockHdr {
    pub src_cid: u64,
    pub dst_cid: u64,
    pub src_port: u32,
    pub dst_port: u32,
    pub len: u32,
    pub type_: u16,
    pub op: u16,
    pub flags: u32,
    pub buf_alloc: u32,
    pub fwd_cnt: u32,
}

impl VirtioVsockHdr {
    #[expect(
        clippy::too_many_arguments,
        reason = "the wire header fields map directly to the virtio-vsock specification"
    )]
    pub const fn new(
        src_cid: u64,
        dst_cid: u64,
        src_port: u32,
        dst_port: u32,
        len: u32,
        op: VirtioVsockOp,
        flags: u32,
        buf_alloc: u32,
        fwd_cnt: u32,
    ) -> Self {
        Self {
            src_cid,
            dst_cid,
            src_port,
            dst_port,
            len,
            type_: VirtioVsockType::Stream as u16,
            op: op as u16,
            flags,
            buf_alloc,
            fwd_cnt,
        }
    }

    pub fn op(&self) -> Option<VirtioVsockOp> {
        VirtioVsockOp::try_from(self.op).ok()
    }
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub(super) enum VirtioVsockEventId {
    TransportReset = 0,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct VirtioVsockEvent {
    pub id: u32,
}
