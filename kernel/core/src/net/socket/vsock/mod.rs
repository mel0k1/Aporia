mod addr;
mod stream;
mod transport;

pub(crate) use addr::VsockSocketAddr;
pub(crate) use stream::VsockStreamSocket;

pub(in crate::net) fn init() {
    transport::init();
}
