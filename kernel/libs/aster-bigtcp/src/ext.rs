use crate::{iface::ScheduleNextPoll, socket::SocketEventObserver};

pub trait Ext {
    type ScheduleNextPoll: ScheduleNextPoll;

    type TcpEventObserver: SocketEventObserver + Clone;

    type UdpEventObserver: SocketEventObserver;
}
