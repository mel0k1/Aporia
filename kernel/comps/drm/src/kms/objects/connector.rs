use alloc::{sync::Weak, vec::Vec};

use ostd::sync::Mutex;

use crate::kms::{
    display_info::DrmDisplayInfo,
    display_mode::DrmDisplayMode,
    objects::{
        KmsObjectId, KmsObjectIndex, KmsObjectMask, encoder::DrmEncoder,
        property::DrmPropertyAttachments,
    },
};

#[derive(Debug)]
pub struct DrmConnector {
    id: KmsObjectId,
    index: KmsObjectIndex,
    config: DrmConnectorConfig,
    state: Mutex<DrmConnectorState>,
    probe_state: Mutex<DrmConnectorProbeState>,
    properties: DrmPropertyAttachments,
}

impl DrmConnector {
    pub(super) fn new(
        id: KmsObjectId,
        index: KmsObjectIndex,
        config: DrmConnectorConfig,
        properties: DrmPropertyAttachments,
    ) -> Self {
        Self {
            id,
            index,
            config,
            state: Mutex::new(DrmConnectorState::default()),
            probe_state: Mutex::new(DrmConnectorProbeState::default()),
            properties,
        }
    }

    pub(crate) fn id(&self) -> KmsObjectId {
        self.id
    }

    pub(crate) fn index(&self) -> KmsObjectIndex {
        self.index
    }

    pub(crate) fn type_(&self) -> DrmConnectorType {
        self.config.type_
    }

    pub(crate) fn type_index(&self) -> u32 {
        self.config.type_index
    }

    pub(crate) fn state(&self) -> &Mutex<DrmConnectorState> {
        &self.state
    }

    pub(crate) fn update_state(&self, state: DrmConnectorState) {
        *self.state.lock() = state;
    }

    pub(crate) fn probe_state(&self) -> &Mutex<DrmConnectorProbeState> {
        &self.probe_state
    }

    pub(crate) fn update_probe_state(&self, probe_state: DrmConnectorProbeState) {
        *self.probe_state.lock() = probe_state;
    }

    pub(crate) fn possible_encoders(&self) -> &KmsObjectMask {
        &self.config.possible_encoders
    }

    pub(crate) fn properties(&self) -> &DrmPropertyAttachments {
        &self.properties
    }
}

#[derive(Debug)]
pub(super) struct DrmConnectorConfig {
    pub type_: DrmConnectorType,
    pub type_index: u32,
    pub possible_encoders: KmsObjectMask,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct DrmConnectorState {
    encoder: Weak<DrmEncoder>,
}

impl DrmConnectorState {
    pub(crate) fn new(encoder: Weak<DrmEncoder>) -> Self {
        Self { encoder }
    }

    pub(crate) fn encoder(&self) -> &Weak<DrmEncoder> {
        &self.encoder
    }
}

#[derive(Clone, Debug)]
pub struct DrmConnectorProbeState {
    status: DrmConnectorStatus,
    display_modes: Vec<DrmDisplayMode>,
    display_info: DrmDisplayInfo,
}

impl DrmConnectorProbeState {
    pub fn new(
        status: DrmConnectorStatus,
        display_modes: Vec<DrmDisplayMode>,
        display_info: DrmDisplayInfo,
    ) -> Self {
        Self {
            status,
            display_modes,
            display_info,
        }
    }

    pub(crate) fn status(&self) -> DrmConnectorStatus {
        self.status
    }

    pub(crate) fn display_modes(&self) -> &[DrmDisplayMode] {
        &self.display_modes
    }

    pub(crate) fn display_info(&self) -> DrmDisplayInfo {
        self.display_info
    }
}

impl Default for DrmConnectorProbeState {
    fn default() -> Self {
        Self {
            status: DrmConnectorStatus::UnknownConnection,
            display_modes: Vec::new(),
            display_info: DrmDisplayInfo::default(),
        }
    }
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum DrmConnectorType {
    Unknown = 0,
    Vga = 1,
    DviI = 2,
    DviD = 3,
    DviA = 4,
    Composite = 5,
    SVideo = 6,
    Lvds = 7,
    Component = 8,
    NinePinDin = 9,
    DisplayPort = 10,
    HdmiA = 11,
    HdmiB = 12,
    Tv = 13,
    Edp = 14,
    Virtual = 15,
    Dsi = 16,
    Dpi = 17,
    Writeback = 18,
    Spi = 19,
    Usb = 20,
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DrmConnectorStatus {
    Connected = 1,
    Disconnected = 2,
    UnknownConnection = 3,
}
