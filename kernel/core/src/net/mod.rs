mod iface;
mod route;
pub(crate) mod socket;
pub(crate) mod uts_ns;

pub(crate) fn init() {
    iface::init();
    route::init();
    socket::netlink::init();
    socket::vsock::init();
}

pub(crate) fn init_in_first_kthread() {
    iface::init_in_first_kthread();
}
