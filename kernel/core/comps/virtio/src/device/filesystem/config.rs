use core::mem::offset_of;

use aster_util::safe_ptr::SafePtr;
use ostd_pod::FromZeros;

use crate::transport::{ConfigManager, VirtioTransport};

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub(super) struct FileSystemFeatures: u64 {
        const NOTIFICATION = 1 << 0;
    }
}

impl FileSystemFeatures {
    pub(super) fn supported_features() -> Self {
        Self::empty()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct VirtioFsConfig {
    tag: [u8; 36],
    num_request_queues: u32,
}

impl VirtioFsConfig {
    pub(super) fn parse_tag(&self) -> &str {
        let len = self
            .tag
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(self.tag.len());

        match core::str::from_utf8(&self.tag[..len]) {
            Ok(tag) => tag,
            Err(_) => "<invalid-tag>",
        }
    }

    pub(super) fn num_request_queues(&self) -> u32 {
        self.num_request_queues
    }

    pub(super) fn new_manager(transport: &dyn VirtioTransport) -> ConfigManager<Self> {
        let safe_ptr = transport
            .device_config_mem()
            .map(|mem| SafePtr::new(mem, 0));
        let bar_space = transport.device_config_bar();

        ConfigManager::new(safe_ptr, bar_space)
    }
}

impl ConfigManager<VirtioFsConfig> {
    pub(super) fn read_config(&self) -> VirtioFsConfig {
        let mut config = VirtioFsConfig::new_zeroed();

        for (index, byte) in config.tag.iter_mut().enumerate() {
            *byte = self
                .read_once::<u8>(offset_of!(VirtioFsConfig, tag) + index)
                .unwrap();
        }
        config.num_request_queues = self
            .read_once::<u32>(offset_of!(VirtioFsConfig, num_request_queues))
            .unwrap();

        config
    }
}
