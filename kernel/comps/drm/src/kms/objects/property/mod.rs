use alloc::{sync::Arc, vec, vec::Vec};

use aster_core::prelude::*;
use aster_util::fixed_str::FixedCStr;

use crate::kms::objects::{DrmKmsObjectType, KmsObjectId, plane::DrmPlaneType};

pub(crate) mod blob;
pub(crate) mod in_formats;

pub(crate) const DRM_PROP_NAME_LEN: usize = 32;
pub(super) type KmsObjectPropValue = u64;

#[derive(Debug)]
pub(crate) struct DrmProperty {
    id: KmsObjectId,
    name: FixedCStr<DRM_PROP_NAME_LEN>,
    value_type: DrmPropertyValueType,
    is_immutable: bool,
    is_atomic: bool,
}

impl DrmProperty {
    pub(crate) fn id(&self) -> KmsObjectId {
        self.id
    }

    pub(crate) fn name(&self) -> &FixedCStr<DRM_PROP_NAME_LEN> {
        &self.name
    }

    pub(crate) fn flags(&self) -> DrmPropertyFlags {
        let mut flags = match &self.value_type {
            DrmPropertyValueType::Plain => DrmPropertyFlags::empty(),
            DrmPropertyValueType::Range { .. } => DrmPropertyFlags::RANGE,
            DrmPropertyValueType::SignedRange { .. } => DrmPropertyFlags::SIGNED_RANGE,
            DrmPropertyValueType::Enum(_) => DrmPropertyFlags::ENUM,
            DrmPropertyValueType::Bitmask(_) => DrmPropertyFlags::BITMASK,
            DrmPropertyValueType::Blob => DrmPropertyFlags::BLOB,
            DrmPropertyValueType::Object(_) => DrmPropertyFlags::OBJECT,
        };
        flags.set(DrmPropertyFlags::IMMUTABLE, self.is_immutable);
        flags.set(DrmPropertyFlags::ATOMIC, self.is_atomic);
        flags
    }

    pub(crate) fn value_type(&self) -> &DrmPropertyValueType {
        &self.value_type
    }

    fn new(
        id: KmsObjectId,
        name: &str,
        flags: DrmPropertyFlags,
        value_type: DrmPropertyValueType,
    ) -> Self {
        Self {
            id,
            name: FixedCStr::from_str_truncated(name),
            value_type,
            is_immutable: flags.contains(DrmPropertyFlags::IMMUTABLE),
            is_atomic: flags.contains(DrmPropertyFlags::ATOMIC),
        }
    }

    fn new_blob(id: KmsObjectId, name: &str, flags: DrmPropertyFlags) -> Self {
        Self::new(id, name, flags, DrmPropertyValueType::Blob)
    }

    fn new_object(
        id: KmsObjectId,
        name: &str,
        flags: DrmPropertyFlags,
        object_type: DrmKmsObjectType,
    ) -> Self {
        Self::new(id, name, flags, DrmPropertyValueType::Object(object_type))
    }

    fn new_bool(id: KmsObjectId, name: &str, flags: DrmPropertyFlags) -> Self {
        Self::new_range(id, name, flags, 0, 1)
    }

    fn new_signed_range(
        id: KmsObjectId,
        name: &str,
        flags: DrmPropertyFlags,
        min: i64,
        max: i64,
    ) -> Self {
        Self::new(
            id,
            name,
            flags,
            DrmPropertyValueType::SignedRange { min, max },
        )
    }

    fn new_range(id: KmsObjectId, name: &str, flags: DrmPropertyFlags, min: u64, max: u64) -> Self {
        Self::new(id, name, flags, DrmPropertyValueType::Range { min, max })
    }

    fn new_enum(
        id: KmsObjectId,
        name: &str,
        flags: DrmPropertyFlags,
        enums: Vec<DrmPropertyEnum>,
    ) -> Self {
        Self::new(id, name, flags, DrmPropertyValueType::Enum(enums))
    }
}

#[derive(Debug, Default)]
pub(crate) struct DrmPropertyAttachments {
    attachments: Vec<DrmPropertyAttachment>,
}

impl DrmPropertyAttachments {
    pub(super) fn attach(
        &mut self,
        property: &Arc<DrmProperty>,
        initial_value: KmsObjectPropValue,
    ) -> Result<()> {
        if self.contains(property.id()) {
            return Err(errno!(
                EEXIST,
                "the property is already attached to the KMS object"
            ));
        }

        self.attachments.push(DrmPropertyAttachment {
            property: Arc::clone(property),
            value: initial_value,
        });
        Ok(())
    }

    pub(crate) fn attachments(&self) -> &[DrmPropertyAttachment] {
        &self.attachments
    }

    fn contains(&self, property_id: KmsObjectId) -> bool {
        self.attachments
            .iter()
            .any(|attachment| attachment.property.id() == property_id)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct DrmPropertyAttachment {
    property: Arc<DrmProperty>,
    value: KmsObjectPropValue,
}

impl DrmPropertyAttachment {
    pub(crate) fn property(&self) -> &Arc<DrmProperty> {
        &self.property
    }

    pub(crate) fn value(&self) -> KmsObjectPropValue {
        self.value
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum DrmStandardProperty {
    PlaneType,
    InFormats,
    SrcX,
    SrcY,
    SrcW,
    SrcH,
    CrtcX,
    CrtcY,
    CrtcW,
    CrtcH,
    FbId,
    CrtcId,
    Active,
    ModeId,
}

impl DrmStandardProperty {
    pub(super) fn create_property(self, id: KmsObjectId) -> DrmProperty {
        match self {
            Self::PlaneType => DrmProperty::new_enum(
                id,
                "type",
                DrmPropertyFlags::IMMUTABLE,
                vec![
                    DrmPropertyEnum::new(DrmPlaneType::Primary as u64, "Primary"),
                    DrmPropertyEnum::new(DrmPlaneType::Overlay as u64, "Overlay"),
                    DrmPropertyEnum::new(DrmPlaneType::Cursor as u64, "Cursor"),
                ],
            ),
            Self::InFormats => DrmProperty::new_blob(id, "IN_FORMATS", DrmPropertyFlags::IMMUTABLE),
            Self::SrcX => {
                DrmProperty::new_range(id, "SRC_X", DrmPropertyFlags::ATOMIC, 0, u32::MAX as u64)
            }
            Self::SrcY => {
                DrmProperty::new_range(id, "SRC_Y", DrmPropertyFlags::ATOMIC, 0, u32::MAX as u64)
            }
            Self::SrcW => {
                DrmProperty::new_range(id, "SRC_W", DrmPropertyFlags::ATOMIC, 0, u32::MAX as u64)
            }
            Self::SrcH => {
                DrmProperty::new_range(id, "SRC_H", DrmPropertyFlags::ATOMIC, 0, u32::MAX as u64)
            }
            Self::CrtcX => DrmProperty::new_signed_range(
                id,
                "CRTC_X",
                DrmPropertyFlags::ATOMIC,
                i32::MIN as i64,
                i32::MAX as i64,
            ),
            Self::CrtcY => DrmProperty::new_signed_range(
                id,
                "CRTC_Y",
                DrmPropertyFlags::ATOMIC,
                i32::MIN as i64,
                i32::MAX as i64,
            ),
            Self::CrtcW => {
                DrmProperty::new_range(id, "CRTC_W", DrmPropertyFlags::ATOMIC, 0, u32::MAX as u64)
            }
            Self::CrtcH => {
                DrmProperty::new_range(id, "CRTC_H", DrmPropertyFlags::ATOMIC, 0, u32::MAX as u64)
            }
            Self::FbId => DrmProperty::new_object(
                id,
                "FB_ID",
                DrmPropertyFlags::ATOMIC,
                DrmKmsObjectType::Framebuffer,
            ),
            Self::CrtcId => DrmProperty::new_object(
                id,
                "CRTC_ID",
                DrmPropertyFlags::ATOMIC,
                DrmKmsObjectType::Crtc,
            ),
            Self::Active => DrmProperty::new_bool(id, "ACTIVE", DrmPropertyFlags::ATOMIC),
            Self::ModeId => DrmProperty::new_blob(id, "MODE_ID", DrmPropertyFlags::ATOMIC),
        }
    }
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug)]
    pub(crate) struct DrmPropertyFlags: u32 {
        const RANGE        = 1 << 1;
        const IMMUTABLE    = 1 << 2;
        const ENUM         = 1 << 3;
        const BLOB         = 1 << 4;
        const BITMASK      = 1 << 5;
        const OBJECT       = 1 << 6;
        const SIGNED_RANGE = 1 << 7;
        const ATOMIC       = 1 << 31;
    }
}

#[derive(Clone, Debug)]
pub(crate) enum DrmPropertyValueType {
    #[expect(dead_code)]
    Plain,
    Range {
        min: u64,
        max: u64,
    },
    SignedRange {
        min: i64,
        max: i64,
    },
    Enum(Vec<DrmPropertyEnum>),
    #[expect(dead_code)]
    Bitmask(Vec<DrmPropertyEnum>),
    Blob,
    Object(DrmKmsObjectType),
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct DrmPropertyEnum {
    value: u64,
    name: FixedCStr<DRM_PROP_NAME_LEN>,
}

impl DrmPropertyEnum {
    fn new(value: u64, name: &str) -> Self {
        Self {
            value,
            name: FixedCStr::from_str_truncated(name),
        }
    }

    pub(crate) fn value(&self) -> u64 {
        self.value
    }
}
