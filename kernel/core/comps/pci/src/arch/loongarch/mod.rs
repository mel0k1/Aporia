use core::{alloc::Layout, ops::RangeInclusive};

use align_ext::AlignExt;
use fdt::node::FdtNode;
use fdt_util::AcquireIoMems;
use ostd::{
    Error, arch::boot::DEVICE_TREE, io::IoMem, mm::VmIoOnce, prelude::Paddr, sync::SpinLock, warn,
};
use spin::Once;

use crate::PciDeviceLocation;

static PCI_ECAM_CFG_SPACE: Once<IoMem> = Once::new();

pub(crate) fn write32(location: &PciDeviceLocation, offset: u32, value: u32) -> Result<(), Error> {
    if offset > PCI_ECAM_MAX_OFFSET {
        return Err(Error::InvalidArgs);
    }
    PCI_ECAM_CFG_SPACE.get().ok_or(Error::IoError)?.write_once(
        (encode_as_address_offset(location) | offset) as usize,
        &value,
    )
}

pub(crate) fn read32(location: &PciDeviceLocation, offset: u32) -> Result<u32, Error> {
    if offset > PCI_ECAM_MAX_OFFSET {
        return Err(Error::InvalidArgs);
    }
    PCI_ECAM_CFG_SPACE
        .get()
        .ok_or(Error::IoError)?
        .read_once((encode_as_address_offset(location) | offset) as usize)
}

const PCI_ECAM_MAX_OFFSET: u32 = 0xffc;

const PCI_ECAM_BUS_SHIFT: u32 = 20;
const PCI_ECAM_DEVICE_SHIFT: u32 = 15;
const PCI_ECAM_FUNCTION_SHIFT: u32 = 12;

fn encode_as_address_offset(location: &PciDeviceLocation) -> u32 {
    ((location.bus as u32) << PCI_ECAM_BUS_SHIFT)
        | ((location.device as u32) << PCI_ECAM_DEVICE_SHIFT)
        | ((location.function as u32) << PCI_ECAM_FUNCTION_SHIFT)
}

pub(crate) fn init() -> Option<RangeInclusive<u8>> {
    let Some(pci) = DEVICE_TREE
        .get()
        .unwrap()
        .find_compatible(&["pci-host-ecam-generic"])
    else {
        warn!("no generic host controller node found in the device tree");
        return None;
    };

    let bus_range = if let Some(prop) = pci.property("bus-range") {
        if prop.value.len() != 8 || prop.value[0..3] != [0, 0, 0] || prop.value[4..7] != [0, 0, 0] {
            warn!(
                "node should have a `bus-range` property with two bytes, but found `{:?}`",
                prop.value
            );
            return None;
        }
        if prop.value[3] != 0 {
            warn!(
                "node with a non-zero bus start `{}` is not supported yet",
                prop.value[3]
            );
            return None;
        }
        prop.value[3]..=prop.value[7]
    } else {
        0..=255
    };

    let io_size = (*bus_range.end() as usize + 1) << PCI_ECAM_BUS_SHIFT;
    let [io_mem] = pci.acquire_io_mems([io_size])?;

    init_mmio_allocator_from_fdt(&pci);

    PCI_ECAM_CFG_SPACE.call_once(|| io_mem);

    Some(bus_range)
}

struct MmioAllocator {
    base: Paddr,
    size: Paddr,
    offset: Paddr,
}

impl MmioAllocator {
    const fn new(base: Paddr, size: Paddr) -> Self {
        MmioAllocator {
            base,
            size,
            offset: 0,
        }
    }

    fn allocate(&mut self, layout: Layout) -> Option<Paddr> {
        let align = layout.align();
        let size = layout.size();

        let current = self.base + self.offset;
        let aligned = current.align_up(align);
        let aligned_offset = aligned - self.base;

        if aligned_offset + size > self.size {
            return None;
        }
        self.offset = aligned_offset + size;
        Some(aligned)
    }
}

static MMIO_ALLOCATOR: Once<SpinLock<MmioAllocator>> = Once::new();

fn init_mmio_allocator_from_fdt(node: &FdtNode) {
    let ranges = node
        .property("ranges")
        .expect("Missing 'ranges' property in PCIe node");
    let data = ranges.value;

    let entry_size = 7 * 4;
    let mut i = 0;

    while i + entry_size <= data.len() {
        let pci_space = u32::from_be_bytes(data[i..i + 4].try_into().unwrap());
        let _pci_addr = u64::from_be_bytes(data[i + 4..i + 12].try_into().unwrap());
        let cpu_addr = u64::from_be_bytes(data[i + 12..i + 20].try_into().unwrap());
        let size = u64::from_be_bytes(data[i + 20..i + 28].try_into().unwrap());

        if (pci_space >> 24) == 0x2 {
            MMIO_ALLOCATOR
                .call_once(|| SpinLock::new(MmioAllocator::new(cpu_addr as usize, size as usize)));
            break;
        }

        i += entry_size;
    }
}

pub(crate) fn alloc_mmio(layout: Layout) -> Option<Paddr> {
    MMIO_ALLOCATOR.get().unwrap().lock().allocate(layout)
}

pub(crate) const MSIX_DEFAULT_MSG_ADDR: u32 = 0x2ff0_0000;

pub(crate) fn construct_remappable_msix_address(_remapping_index: u32) -> u32 {
    unimplemented!()
}
