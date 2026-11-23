use ostd_pod::IntoBytes;
use smoltcp::wire::{IpProtocol, IpRepr, UdpRepr};

use crate::{
    iface::wire::utils::{Be16, Checksum},
    packet::{ApplicationLayer, RxPacket, TransportLayer, TxPacket},
};

pub(in crate::iface) fn parse(
    mut pkt: RxPacket<TransportLayer>,
    ip_repr: &IpRepr,
    csum: bool,
) -> Option<(RxPacket<ApplicationLayer>, UdpRepr)> {
    let header = pkt.reader().read_val::<Header>().ok()?;

    let packet_len = u16::from(header.len) as usize;
    if packet_len < size_of::<Header>() || packet_len > pkt.len() {
        return None;
    }
    pkt.truncate(packet_len);

    let dst_port = u16::from(header.dst);
    if dst_port == 0 {
        return None;
    }

    if header.csum == 0 {
        if csum && matches!(ip_repr, IpRepr::Ipv6(_)) {
            return None;
        }
    } else if csum
        && Checksum::new()
            .with_pseudo_header(ip_repr, packet_len)
            .with_reader(pkt.reader())
            .finish()
            != u16::MAX
    {
        return None;
    }

    let repr = UdpRepr {
        src_port: header.src.into(),
        dst_port,
    };
    Some((pkt.peel(size_of::<Header>()), repr))
}

pub(in crate::iface) fn emit(
    pkt: TxPacket<ApplicationLayer>,
    ip_repr: &IpRepr,
    udp_repr: &UdpRepr,
    csum: bool,
) -> TxPacket<TransportLayer> {
    debug_assert_eq!(ip_repr.next_header(), IpProtocol::Udp);

    let packet_len = size_of::<Header>() + pkt.len();
    debug_assert_eq!(ip_repr.payload_len(), packet_len);

    let mut header = Header {
        src: udp_repr.src_port.into(),
        dst: udp_repr.dst_port.into(),
        len: (packet_len as u16).into(),
        csum: 0,
    };

    if csum {
        let csum_val = !Checksum::new()
            .with_pseudo_header(ip_repr, packet_len)
            .with_bytes(header.as_bytes())
            .with_reader(pkt.reader())
            .finish();
        header.csum = if csum_val == 0 { u16::MAX } else { csum_val };
    }

    pkt.prepend_and_pack(&header)
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct Header {
    src: Be16,
    dst: Be16,
    len: Be16,
    csum: u16,
}
