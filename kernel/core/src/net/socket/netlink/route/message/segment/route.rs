use crate::prelude::*;

#[expect(unused)]
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct CRtMsg {
    family: u8,
    dst_len: u8,
    src_len: u8,
    tos: u8,
    table: u8,
    protocol: u8,
    scope: u8,
    type_: u8,
    flags: u32,
}
