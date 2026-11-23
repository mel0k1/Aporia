use core::mem::offset_of;

use aster_util::safe_ptr::SafePtr;
use ostd_pod::FromZeros;

use crate::transport::{ConfigManager, VirtioTransport};

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub(super) struct ConsoleFeatures: u64 {
        const VIRTIO_CONSOLE_F_SIZE        = 1 << 0;
        const VIRTIO_CONSOLE_F_MULTIPORT   = 1 << 1;
        const VIRTIO_CONSOLE_F_EMERG_WRITE = 1 << 2;
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct VirtioConsoleConfig {
    pub cols: u16,
    pub rows: u16,
    pub max_nr_ports: u32,
    pub emerg_wr: u32,
}

impl VirtioConsoleConfig {
    pub(super) fn new_manager(transport: &dyn VirtioTransport) -> ConfigManager<Self> {
        let safe_ptr = transport
            .device_config_mem()
            .map(|mem| SafePtr::new(mem, 0));
        let bar_space = transport.device_config_bar();
        ConfigManager::new(safe_ptr, bar_space)
    }
}

impl ConfigManager<VirtioConsoleConfig> {
    pub(super) fn read_config(&self) -> VirtioConsoleConfig {
        let mut console_config = VirtioConsoleConfig::new_zeroed();
        console_config.cols = self
            .read_once::<u16>(offset_of!(VirtioConsoleConfig, cols))
            .unwrap();
        console_config.rows = self
            .read_once::<u16>(offset_of!(VirtioConsoleConfig, rows))
            .unwrap();
        console_config.max_nr_ports = self
            .read_once::<u32>(offset_of!(VirtioConsoleConfig, max_nr_ports))
            .unwrap();

        console_config
    }

    #[expect(dead_code)]
    pub(super) fn emerg_write(&self, value: u32) {
        if self.is_modern() {
            self.write_once(offset_of!(VirtioConsoleConfig, emerg_wr), value)
                .unwrap();
        }
    }
}
