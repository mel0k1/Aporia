use core::mem::offset_of;

use aster_util::safe_ptr::SafePtr;
use bitflags::bitflags;
use ostd_pod::FromZeros;

use crate::transport::{ConfigManager, VirtioTransport};

bitflags! {
    /// Virtio network feature bits.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub(super) struct NetworkFeatures: u64 {
        const VIRTIO_NET_F_CSUM                = 1 << 0;
        const VIRTIO_NET_F_GUEST_CSUM          = 1 << 1;
        const VIRTIO_NET_F_CTRL_GUEST_OFFLOADS = 1 << 2;
        const VIRTIO_NET_F_MTU                 = 1 << 3;
        const VIRTIO_NET_F_MAC                 = 1 << 5;
        const VIRTIO_NET_F_GUEST_TSO4          = 1 << 7;
        const VIRTIO_NET_F_GUEST_TSO6          = 1 << 8;
        const VIRTIO_NET_F_GUEST_ECN           = 1 << 9;
        const VIRTIO_NET_F_GUEST_UFO           = 1 << 10;
        const VIRTIO_NET_F_HOST_TSO4           = 1 << 11;
        const VIRTIO_NET_F_HOST_TSO6           = 1 << 12;
        const VIRTIO_NET_F_HOST_ECN            = 1 << 13;
        const VIRTIO_NET_F_HOST_UFO            = 1 << 14;
        const VIRTIO_NET_F_MRG_RXBUF           = 1 << 15;
        const VIRTIO_NET_F_STATUS              = 1 << 16;
        const VIRTIO_NET_F_CTRL_VQ             = 1 << 17;
        const VIRTIO_NET_F_CTRL_RX             = 1 << 18;
        const VIRTIO_NET_F_CTRL_VLAN           = 1 << 19;
        const VIRTIO_NET_F_EXTRA               = 1 << 20;
        const VIRTIO_NET_F_GUEST_ANNOUNCE      = 1 << 21;
        const VIRTIO_NET_F_MQ                  = 1 << 22;
        const VIRTIO_NET_F_CTRL_MAC_ADDR       = 1 << 23;
        const VIRTIO_NET_F_HASH_TUNNEL         = 1 << 51;
        const VIRTIO_NET_F_VQ_NOTF_COAL        = 1 << 52;
        const VIRTIO_NET_F_NOTF_COAL           = 1 << 53;
        const VIRTIO_NET_F_GUEST_USO4          = 1 << 54;
        const VIRTIO_NET_F_GUEST_USO6          = 1 << 55;
        const VIRTIO_NET_F_HOST_USO            = 1 << 56;
        const VIRTIO_NET_F_HASH_REPORT         = 1 << 57;
        const VIRTIO_NET_F_GUEST_HDRLEN        = 1 << 59;
        const VIRTIO_NET_F_RSS                 = 1 << 60;
        const VIRTIO_NET_F_RSC_EXT             = 1 << 61;
        const VIRTIO_NET_F_STANDBY             = 1 << 62;
        const VIRTIO_NET_F_SPEED_DUPLEX        = 1 << 63;
    }
}

impl NetworkFeatures {
    pub(super) fn supported_features() -> Self {
        NetworkFeatures::VIRTIO_NET_F_MAC | NetworkFeatures::VIRTIO_NET_F_STATUS
    }
}

/// Virtio network device status.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct Status(u16);

bitflags! {
    impl Status: u16 {
        const VIRTIO_NET_S_LINK_UP  = 1;
        const VIRTIO_NET_S_ANNOUNCE = 2;



        const _                     = !0;
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct VirtioNetConfig {
    pub mac: [u8; 6],
    pub status: Status,
    max_virtqueue_pairs: u16,
    pub mtu: u16,
    speed: u32,
    duplex: u8,
    rss_max_key_size: u8,
    rss_max_indirection_table_length: u16,
    supported_hash_types: u32,
}

impl VirtioNetConfig {
    pub(super) fn new_manager(transport: &dyn VirtioTransport) -> ConfigManager<Self> {
        let safe_ptr = transport
            .device_config_mem()
            .map(|mem| SafePtr::new(mem, 0));
        let bar_space = transport.device_config_bar();
        ConfigManager::new(safe_ptr, bar_space)
    }
}

impl ConfigManager<VirtioNetConfig> {
    pub(super) fn read_config(&self) -> VirtioNetConfig {
        let mut net_config = VirtioNetConfig::new_zeroed();

        for (i, mac_byte) in net_config.mac.iter_mut().enumerate() {
            *mac_byte = self
                .read_once::<u8>(offset_of!(VirtioNetConfig, mac) + i)
                .unwrap();
        }
        net_config.status = Status::from_bits_retain(
            self.read_once::<u16>(offset_of!(VirtioNetConfig, status))
                .unwrap(),
        );

        if self.is_modern() {
            net_config.max_virtqueue_pairs = self
                .read_once::<u16>(offset_of!(VirtioNetConfig, max_virtqueue_pairs))
                .unwrap();
            net_config.mtu = self
                .read_once::<u16>(offset_of!(VirtioNetConfig, mtu))
                .unwrap();
            net_config.speed = self
                .read_once::<u32>(offset_of!(VirtioNetConfig, speed))
                .unwrap();
            net_config.duplex = self
                .read_once::<u8>(offset_of!(VirtioNetConfig, duplex))
                .unwrap();
            net_config.rss_max_key_size = self
                .read_once::<u8>(offset_of!(VirtioNetConfig, rss_max_key_size))
                .unwrap();
            net_config.rss_max_indirection_table_length = self
                .read_once::<u16>(offset_of!(
                    VirtioNetConfig,
                    rss_max_indirection_table_length
                ))
                .unwrap();
            net_config.supported_hash_types = self
                .read_once::<u32>(offset_of!(VirtioNetConfig, supported_hash_types))
                .unwrap();
        }

        net_config
    }
}
