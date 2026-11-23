use crate::{
    net::socket::netlink::{
        message::{CMsgSegHdr, DoneSegment, ProtocolSegment, SegHdrCommonFlags},
        route::message::RtnlSegment,
    },
    prelude::*,
};

#[derive(Eq, PartialEq)]
pub(super) enum ResponseMode {
    Single,
    Dump,
    FilteredDump,
}

pub(super) fn finish_response(
    request_header: &CMsgSegHdr,
    mode: ResponseMode,
    response_segments: &mut Vec<RtnlSegment>,
) {
    let response_flags = match mode {
        ResponseMode::Single => {
            assert_eq!(response_segments.len(), 1);
            return;
        }
        ResponseMode::Dump => SegHdrCommonFlags::MULTI,
        ResponseMode::FilteredDump => SegHdrCommonFlags::MULTI | SegHdrCommonFlags::DUMP_FILTERED,
    };

    append_done_segment(request_header, response_segments);
    for segment in response_segments.iter_mut() {
        let header = segment.header_mut();
        header.flags |= response_flags.bits();
    }
}

fn append_done_segment(request_header: &CMsgSegHdr, response_segments: &mut Vec<RtnlSegment>) {
    let done_segment = DoneSegment::new_from_request(request_header, None);
    response_segments.push(RtnlSegment::Done(done_segment));
}
