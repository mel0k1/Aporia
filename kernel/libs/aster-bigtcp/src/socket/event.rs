pub trait SocketEventObserver: Send + Sync {
    fn on_events(&self, events: SocketEvents);
}

impl SocketEventObserver for () {
    fn on_events(&self, _events: SocketEvents) {}
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub struct SocketEvents: u8 {
        const CAN_RECV    = 1;
        const CAN_SEND    = 2;
        const CLOSED_RECV = 4;
        const CLOSED_SEND = 8;
    }
}
