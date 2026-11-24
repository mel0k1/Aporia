pub mod device;

use aster_util::safe_ptr::SafePtr;
use ostd::io::IoMem;

use crate::transport::VirtioTransport;

pub const DEVICE_NAME: &str = "Virtio-Input";

/// Select value used for `device::InputDevice::select_config`.
#[repr(u8)]
#[derive(Clone, Copy, Debug)]
enum InputConfigSelect {
    /// Invalid configuration selection.
    #[expect(dead_code)]
    Unset = 0x00,
    /// Returns the name of the device, subsel is zero.
    IdName = 0x01,
    /// Returns the serial number of the device, subsel is zero.
    #[expect(dead_code)]
    IdSerial = 0x02,
    /// Returns ID information of the device, subsel is zero.
    #[expect(dead_code)]
    IdDevids = 0x03,
    /// Returns input properties of the device, subsel is zero.
    /// Individual bits in the bitmap correspond to INPUT_PROP_* constants used
    /// by the underlying evdev implementation.
    PropBits = 0x10,
    /// subsel specifies the event type using EV_* constants in the underlying
    /// evdev implementation. If size is non-zero the event type is supported
    /// and a bitmap of supported event codes is returned. Individual
    /// bits in the bitmap correspond to implementation-defined input event codes,
    /// for example keys or pointing device axes.
    EvBits = 0x11,
    /// subsel specifies the absolute axis using ABS_* constants in the underlying
    /// evdev implementation. Information about the axis will be returned.
    #[expect(dead_code)]
    AbsInfo = 0x12,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct VirtioInputConfig {
    /// write only
    select: u8,
    /// write only
    subsel: u8,
    /// read only
    size: u8,
    _reversed: [u8; 5],
    /// read only
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

/// Both queues use the same `virtio_input_event` struct. `type`, `code` and `value`
/// are filled according to the Linux input layer (evdev) interface.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct VirtioInputEvent {
    /// Event type.
    pub event_type: u16,
    /// Event code.
    pub code: u16,
    /// Event value.
    pub value: u32,
}

const QUEUE_EVENT: u16 = 0;
const QUEUE_STATUS: u16 = 1;
