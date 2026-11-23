use core::ops::Range;

use id_alloc::IdAlloc;
use spin::Once;

use super::IoPort;
use crate::{
    debug,
    io::RawIoPortRange,
    sync::{LocalIrqDisabled, SpinLock},
};

pub(super) struct IoPortAllocator {
    allocator: SpinLock<IdAlloc, LocalIrqDisabled>,
}

impl IoPortAllocator {
    pub(super) fn acquire<T, A>(&self, port: u16, is_overlapping: bool) -> Option<IoPort<T, A>> {
        let max_io_port_for_t = const {
            assert!(1 <= size_of::<T>() && size_of::<T>() <= 4);
            crate::arch::io::MAX_IO_PORT as usize + 1 - size_of::<T>()
        };
        let port = port as usize;
        if port > max_io_port_for_t {
            return None;
        }

        let range = if !is_overlapping {
            port..port + size_of::<T>()
        } else {
            port..port + 1
        };
        debug!("Try to acquire PIO range: {:#x?}", range);

        let mut allocator = self.allocator.lock();
        if range.clone().any(|i| allocator.is_allocated(i)) {
            return None;
        }

        for i in range.clone() {
            allocator.alloc_specific(i);
        }

        unsafe { Some(IoPort::new_overlapping(port as u16, is_overlapping)) }
    }

    pub(super) unsafe fn recycle(&self, range: Range<usize>) {
        debug!("Recycling PIO range: {:#x?}", range);

        self.allocator.lock().free_consecutive(range);
    }
}

pub(super) static IO_PORT_ALLOCATOR: Once<IoPortAllocator> = Once::new();

pub(in crate::io) unsafe fn init() {
    let max_nr_io_ports = crate::arch::io::MAX_IO_PORT as usize + 1;
    let mut allocator = IdAlloc::with_capacity(max_nr_io_ports);

    unsafe extern "C" {
        fn __sensitive_io_ports_start();
        fn __sensitive_io_ports_end();
    }
    let start = __sensitive_io_ports_start as *const () as usize;
    let end = __sensitive_io_ports_end as *const () as usize;
    assert!((end - start).is_multiple_of(size_of::<RawIoPortRange>()));

    let io_port_range_count = (end - start) / size_of::<RawIoPortRange>();
    for i in 0..io_port_range_count {
        let range_base_addr =
            __sensitive_io_ports_start as *const () as usize + i * size_of::<RawIoPortRange>();
        let port_range = unsafe { *(range_base_addr as *const RawIoPortRange) };

        assert!(port_range.begin < port_range.end);
        debug!("Removing sensitive I/O port range: {:#x?}", port_range);

        for i in port_range.begin..port_range.end {
            allocator.alloc_specific(i as usize);
        }
    }

    IO_PORT_ALLOCATOR.call_once(|| IoPortAllocator {
        allocator: SpinLock::new(allocator),
    });
}

#[cfg(ktest)]
mod test {
    use crate::{arch::device::io_port::ReadWriteAccess, prelude::*};

    type IoPort = crate::io::IoPort<u32, ReadWriteAccess>;
    type ByteIoPort = crate::io::IoPort<u8, ReadWriteAccess>;

    #[ktest]
    fn illegal_region() {
        let io_port_a = IoPort::acquire(0xffff);
        assert!(io_port_a.is_err());

        let io_port_overlapping = IoPort::acquire_overlapping(0xfffe);
        assert!(io_port_overlapping.is_err());
    }

    #[ktest]
    fn conflict_region() {
        let io_port_a = IoPort::acquire(0x60);
        assert!(io_port_a.is_ok());

        let io_port_b = IoPort::acquire(0x62);
        assert!(io_port_b.is_err());

        drop(io_port_a);

        let io_port_b = IoPort::acquire(0x62);
        assert!(io_port_b.is_ok());
    }

    #[ktest]
    fn overlapping_region() {
        let pci_data = IoPort::acquire_overlapping(0xcf8);
        let rst_ctrl = ByteIoPort::acquire(0xcf9);
        assert!(pci_data.is_ok());
        assert!(rst_ctrl.is_ok());

        let pci_data2 = IoPort::acquire_overlapping(0xcf8);
        let rst_ctrl2 = ByteIoPort::acquire(0xcf9);
        assert!(pci_data2.is_err());
        assert!(rst_ctrl2.is_err());

        drop(pci_data);
        drop(rst_ctrl);

        let rst_ctrl3 = ByteIoPort::acquire(0xcf9);
        assert!(rst_ctrl3.is_ok());

        let pci_data3 = IoPort::acquire_overlapping(0xcf8);
        assert!(pci_data3.is_ok());
    }

    #[ktest]
    fn last_region() {
        let io_port = IoPort::acquire(0xfffc).unwrap();
        assert!(ByteIoPort::acquire(0xffff).is_err());
        drop(io_port);

        let io_port = ByteIoPort::acquire_overlapping(0xffff).unwrap();
        assert!(ByteIoPort::acquire(0xffff).is_err());
        drop(io_port);

        assert!(ByteIoPort::acquire(0xffff).is_ok());
    }
}
