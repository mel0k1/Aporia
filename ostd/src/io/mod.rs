pub(crate) mod io_mem;

pub use self::io_mem::IoMem;
#[cfg_attr(target_arch = "loongarch64", expect(unused_imports))]
pub(crate) use self::io_mem::{IoMemAllocatorBuilder, Sensitive};

cfg_select! {
    target_arch = "x86_64" => {
        mod io_port;

        pub use self::io_port::IoPort;
        pub(crate) use self::io_port::{RawIoPortRange, reserve_io_port_range, sensitive_io_port};
    }
    _ => {}
}

pub(crate) unsafe fn init(io_mem_builder: IoMemAllocatorBuilder) {
    unsafe { io_mem::init(io_mem_builder) };

    #[cfg(target_arch = "x86_64")]
    unsafe {
        io_port::init()
    };
}
