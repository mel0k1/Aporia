use alloc::{boxed::Box, vec};

pub(super) type RawTcpSocket = smoltcp::socket::tcp::Socket<'static>;
pub type RawUdpSocket = smoltcp::socket::udp::Socket<'static>;

pub(super) fn new_tcp_socket() -> Box<RawTcpSocket> {
    let raw_tcp_socket = {
        let rx_buffer = smoltcp::socket::tcp::SocketBuffer::new(vec![0u8; TCP_RECV_BUF_LEN]);
        let tx_buffer = smoltcp::socket::tcp::SocketBuffer::new(vec![0u8; TCP_SEND_BUF_LEN]);
        RawTcpSocket::new(rx_buffer, tx_buffer)
    };
    Box::new(raw_tcp_socket)
}

pub(super) fn new_udp_socket() -> Box<RawUdpSocket> {
    let raw_udp_socket = {
        let metadata = smoltcp::socket::udp::PacketMetadata::EMPTY;
        let rx_buffer = smoltcp::socket::udp::PacketBuffer::new(
            vec![metadata; UDP_METADATA_LEN],
            vec![0u8; UDP_RECV_PAYLOAD_LEN],
        );
        let tx_buffer = smoltcp::socket::udp::PacketBuffer::new(
            vec![metadata; UDP_METADATA_LEN],
            vec![0u8; UDP_SEND_PAYLOAD_LEN],
        );
        RawUdpSocket::new(rx_buffer, tx_buffer)
    };
    Box::new(raw_udp_socket)
}

pub const TCP_RECV_BUF_LEN: usize = 65536 * 2;
pub const TCP_SEND_BUF_LEN: usize = 65536 * 2;

pub const UDP_SEND_PAYLOAD_LEN: usize = 65536;
pub const UDP_RECV_PAYLOAD_LEN: usize = 65536;
const UDP_METADATA_LEN: usize = 256;
