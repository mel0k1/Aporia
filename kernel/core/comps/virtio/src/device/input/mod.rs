pub mod device;

use aster_util::safe_ptr::SafePtr;
use ostd::io::IoMem;

use crate::transport::VirtioTransport;

pub const DEVICE_NAME: &str = "Virtio-Input";

#[repr(u8)]
#[derive(Clone, Copy, Debug)]
enum InputConfigSelect {
    #[expect(dead_code)]
    Unset = 0x00,
    IdName = 0x01,
    #[expect(dead_code)]
    IdSerial = 0x02,
    #[expect(dead_code)]
    IdDevids = 0x03,
    PropBits = 0x10,
    EvBits = 0x11,
    #[expect(dead_code)]
    AbsInfo = 0x12,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct VirtioInputConfig {
    select: u8,
    subsel: u8,
    size: u8,
    _reversed: [u8; 5],
    data: [u8; 128],
}

impl VirtioInputConfig {
    pub(self) fn new(transport: &dyn VirtioTransport) -> SafePtr<Self, IoMem> {
        let memory = transport.device_config_mem().unwrap();
        SafePtr::new(memory, 0)
    }
}

#[expect(dead_code)]
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct AbsInfo {
    min: u32,
    max: u32,
    fuzz: u32,
    flat: u32,
    res: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct VirtioInputEvent {
    pub event_type: u16,
    pub code: u16,
    pub value: u32,
}

const QUEUE_EVENT: u16 = 0;
const QUEUE_STATUS: u16 = 1;
