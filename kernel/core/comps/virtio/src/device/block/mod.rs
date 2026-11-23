pub mod device;

use core::mem::offset_of;

use aster_block::SECTOR_SIZE;
use aster_util::safe_ptr::SafePtr;
use bitflags::bitflags;
use int_to_c_enum::TryFromInt;
use ostd_pod::FromZeros;

use crate::transport::{ConfigManager, VirtioTransport};

pub const DEVICE_NAME: &str = "Virtio-Block";

bitflags! {
    #[derive(Clone, Copy, Debug)]
    struct BlockFeatures: u64 {
        const BARRIER       = 1 << 0;
        const SIZE_MAX      = 1 << 1;
        const SEG_MAX       = 1 << 2;
        const GEOMETRY      = 1 << 4;
        const RO            = 1 << 5;
        const BLK_SIZE      = 1 << 6;
        const SCSI          = 1 << 7;
        const FLUSH         = 1 << 9;
        const TOPOLOGY      = 1 << 10;
        const CONFIG_WCE    = 1 << 11;
        const MQ            = 1 << 12;
        const DISCARD       = 1 << 13;
        const WRITE_ZEROES  = 1 << 14;

        const ALL_SUPPORTED = Self::BLK_SIZE.bits() | Self::FLUSH.bits();
    }
}

impl BlockFeatures {
    pub(super) fn negotiated_with_device(device_features: u64) -> Self {
        BlockFeatures::from_bits_truncate(device_features) & Self::ALL_SUPPORTED
    }
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, TryFromInt)]
enum ReqType {
    In = 0,
    Out = 1,
    Flush = 4,
    GetId = 8,
    Discard = 11,
    WriteZeroes = 13,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
enum RespStatus {
    Ok = 0,
    IoErr = 1,
    Unsupported = 2,
    _NotReady = 3,
}

#[padding_struct]
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct VirtioBlockConfig {
    capacity: u64,
    size_max: u32,
    seg_max: u32,
    geometry: VirtioBlockGeometry,
    blk_size: u32,
    topology: VirtioBlockTopology,
    writeback: u8,
    unused0: u8,
    num_queues: u16,
    max_discard_sectors: u32,
    max_discard_seg: u32,
    discard_sector_alignment: u32,
    max_write_zeroes_sectors: u32,
    max_write_zeroes_seg: u32,
    write_zeros_may_unmap: u8,
    unused1: [u8; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct VirtioBlockGeometry {
    cylinders: u16,
    heads: u8,
    sectors: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct VirtioBlockTopology {
    physical_block_exp: u8,
    alignment_offset: u8,
    min_io_size: u16,
    opt_io_size: u32,
}

impl VirtioBlockConfig {
    pub(self) fn new_manager(transport: &dyn VirtioTransport) -> ConfigManager<Self> {
        let safe_ptr = transport
            .device_config_mem()
            .map(|mem| SafePtr::new(mem, 0));
        let bar_space = transport.device_config_bar();

        ConfigManager::new(safe_ptr, bar_space)
    }

    pub(self) const fn sector_size() -> usize {
        SECTOR_SIZE
    }
}

impl ConfigManager<VirtioBlockConfig> {
    pub(self) fn read_config(&self) -> VirtioBlockConfig {
        let mut blk_config = VirtioBlockConfig::new_zeroed();
        let cap_low = self
            .read_once::<u32>(offset_of!(VirtioBlockConfig, capacity))
            .unwrap() as u64;
        let cap_high = self
            .read_once::<u32>(offset_of!(VirtioBlockConfig, capacity) + 4)
            .unwrap() as u64;
        blk_config.capacity = (cap_high << 32) | cap_low;
        blk_config.size_max = self
            .read_once::<u32>(offset_of!(VirtioBlockConfig, size_max))
            .unwrap();
        blk_config.seg_max = self
            .read_once::<u32>(offset_of!(VirtioBlockConfig, seg_max))
            .unwrap();
        blk_config.geometry.cylinders = self
            .read_once::<u16>(
                offset_of!(VirtioBlockConfig, geometry)
                    + offset_of!(VirtioBlockGeometry, cylinders),
            )
            .unwrap();
        blk_config.geometry.heads = self
            .read_once::<u8>(
                offset_of!(VirtioBlockConfig, geometry) + offset_of!(VirtioBlockGeometry, heads),
            )
            .unwrap();
        blk_config.geometry.sectors = self
            .read_once::<u8>(
                offset_of!(VirtioBlockConfig, geometry) + offset_of!(VirtioBlockGeometry, sectors),
            )
            .unwrap();
        blk_config.blk_size = self
            .read_once::<u32>(offset_of!(VirtioBlockConfig, blk_size))
            .unwrap();

        if self.is_modern() {}

        blk_config
    }

    pub(self) fn block_size(&self) -> usize {
        self.read_once::<u32>(offset_of!(VirtioBlockConfig, blk_size))
            .unwrap() as usize
    }

    pub(self) fn capacity_sectors(&self) -> usize {
        let cap_low = self
            .read_once::<u32>(offset_of!(VirtioBlockConfig, capacity))
            .unwrap() as usize;
        let cap_high = self
            .read_once::<u32>(offset_of!(VirtioBlockConfig, capacity) + 4)
            .unwrap() as usize;

        (cap_high << 32) | cap_low
    }
}
