use alloc::borrow::ToOwned;
use core::{net::IpAddr, num::NonZeroU32};

use aster_bigtcp::{iface::InterfaceFlags, wire::IpCidr};

use super::util::{self, ResponseMode};
use crate::{
    net::{
        iface::{Iface, iter_all_ifaces},
        socket::netlink::{
            message::{CMsgSegHdr, CSegmentType, GetRequestFlags, SegHdrCommonFlags},
            route::message::{
                AddrAttr, AddrAttrClass, AddrMessageFlags, AddrProtocol, AddrSegment, CIfaddrMsg,
                RtScope, RtnlSegment,
            },
        },
    },
    prelude::*,
    util::net::CSocketAddrFamily,
};

pub(super) fn do_get_addr(
    request_segment: &AddrSegment,
    strict_check: bool,
) -> Result<Vec<RtnlSegment>> {
    let dump_all = {
        let flags = GetRequestFlags::from_bits_truncate(request_segment.header().flags);
        flags.contains(GetRequestFlags::DUMP)
    };
    if !dump_all {
        return Err(errno!(EOPNOTSUPP, "GETADDR only supports dump requests"));
    }

    validate_getaddr_request(request_segment, strict_check)?;

    let requested_family = request_segment.body().family;
    let requested_index = strict_check
        .then(|| NonZeroU32::new(request_segment.body().index))
        .flatten();
    let mut ifaces = iter_all_ifaces()
        .filter(|iface| requested_index.is_none_or(|index| index.get() == iface.index()))
        .peekable();
    if requested_index.is_some() && ifaces.peek().is_none() {
        return Err(errno!(ENODEV, "the interface does not exist"));
    }

    let mut addr_segments: Vec<AddrSegment> = ifaces
        .flat_map(|iface| iface_to_new_addrs(request_segment.header(), requested_family, iface))
        .collect();

    addr_segments.sort_by_key(|segment| segment.body().family);

    let mut response_segments: Vec<RtnlSegment> = addr_segments
        .into_iter()
        .map(RtnlSegment::NewAddr)
        .collect();

    let response_mode = if requested_index.is_some() {
        ResponseMode::FilteredDump
    } else {
        ResponseMode::Dump
    };
    util::finish_response(
        request_segment.header(),
        response_mode,
        &mut response_segments,
    );

    Ok(response_segments)
}

fn validate_getaddr_request(request_segment: &AddrSegment, strict_check: bool) -> Result<()> {
    if !strict_check {
        return Ok(());
    }

    let body = request_segment.body();
    if body.prefix_len != 0 || body.flags != 0 || body.scope != 0 {
        return Err(errno!(
            EINVAL,
            "the address request header contains nonzero prefix length, flags, or scope"
        ));
    }

    const ALLOWED: AddrAttrClass = AddrAttrClass::TARGET_NETNSID;
    if request_segment
        .attrs()
        .iter()
        .any(|attr| attr.class() != ALLOWED)
    {
        return Err(errno!(
            EINVAL,
            "the attribute is not allowed in an address dump request"
        ));
    }
    Ok(())
}

fn iface_to_new_addrs(
    request_header: &CMsgSegHdr,
    requested_family: u8,
    iface: &Arc<Iface>,
) -> impl IntoIterator<Item = AddrSegment> {
    let mut addr_segments = [None, None];

    let dump_ipv4 = requested_family != CSocketAddrFamily::AF_INET6 as u8;
    let dump_ipv6 = requested_family != CSocketAddrFamily::AF_INET as u8;

    if dump_ipv4 && let Some(cidr) = iface.ipv4_cidr() {
        addr_segments[0] = Some(iface_to_new_addr(request_header, iface, IpCidr::Ipv4(cidr)));
    };

    if dump_ipv6 && let Some(cidr) = iface.ipv6_cidr() {
        addr_segments[1] = Some(iface_to_new_addr(request_header, iface, IpCidr::Ipv6(cidr)));
    }

    addr_segments.into_iter().flatten()
}

fn iface_to_new_addr(
    request_header: &CMsgSegHdr,
    iface: &Arc<Iface>,
    ip_cidr: IpCidr,
) -> AddrSegment {
    let (family, address, prefix_len, scope) = match ip_cidr {
        IpCidr::Ipv4(ipv4_cidr) => {
            let ipv4_addr = ipv4_cidr.address();
            let scope = if ipv4_addr.is_loopback() {
                RtScope::HOST
            } else {
                RtScope::UNIVERSE
            };
            (
                CSocketAddrFamily::AF_INET,
                ipv4_addr.into(),
                ipv4_cidr.prefix_len(),
                scope,
            )
        }
        IpCidr::Ipv6(ipv6_cidr) => {
            let ipv6_addr = ipv6_cidr.address();
            let scope = if ipv6_addr.is_loopback() {
                RtScope::HOST
            } else if ipv6_addr.is_unicast_link_local() {
                RtScope::LINK
            } else {
                RtScope::UNIVERSE
            };
            (
                CSocketAddrFamily::AF_INET6,
                ipv6_addr.into(),
                ipv6_cidr.prefix_len(),
                scope,
            )
        }
    };

    let header = CMsgSegHdr {
        len: 0,
        type_: CSegmentType::NEWADDR as _,
        flags: SegHdrCommonFlags::empty().bits(),
        seq: request_header.seq,
        pid: request_header.pid,
    };

    let flags = AddrMessageFlags::PERMANENT;
    let addr_message = CIfaddrMsg {
        family: family as _,
        prefix_len,
        flags: flags.bits() as u8,
        scope: scope as u8,
        index: iface.index(),
    };

    let attrs = match address {
        IpAddr::V4(_) => {
            let mut attrs = vec![AddrAttr::Address(address), AddrAttr::Local(address)];
            if iface.flags().contains(InterfaceFlags::BROADCAST)
                && let Some(broadcast_address) = iface.broadcast_addr()
            {
                attrs.push(AddrAttr::Broadcast(broadcast_address));
            }
            attrs.extend([
                AddrAttr::Label(iface.name().as_cstr().to_owned()),
                AddrAttr::Flags(flags),
            ]);
            attrs
        }
        IpAddr::V6(_) => {
            let mut attrs = vec![AddrAttr::Address(address), AddrAttr::Flags(flags)];
            if address.is_loopback() {
                attrs.push(AddrAttr::Protocol(AddrProtocol::KernelLoopback));
            }
            attrs
        }
    };

    AddrSegment::new(header, addr_message, attrs)
}
