#![short_vis_path::add(overlayfs)]

use super::{
    capabilities::{UpperFilesystemCapabilities, WhiteoutCapability},
    inuse::Uuid,
};
use crate::fs::vfs::xattr::XattrNamespace;

pub(in overlayfs) struct MountPolicy {
    is_effective_read_only: bool,
    uuid: Option<Uuid>,
    upper_capabilities: Option<UpperFilesystemCapabilities>,
    xattr_namespace: XattrNamespace,
}

impl MountPolicy {
    pub(super) fn assemble(
        is_effective_read_only: bool,
        xattr_namespace: XattrNamespace,
        uuid: Option<Uuid>,
        upper_capabilities: Option<UpperFilesystemCapabilities>,
    ) -> Self {
        Self {
            is_effective_read_only,
            uuid,
            upper_capabilities,
            xattr_namespace,
        }
    }

    pub(in overlayfs) fn is_effective_read_only(&self) -> bool {
        self.is_effective_read_only
    }

    pub(in overlayfs) fn xattr_namespace(&self) -> XattrNamespace {
        self.xattr_namespace
    }

    pub(in super::super) fn uuid(&self) -> Option<&Uuid> {
        self.uuid.as_ref()
    }

    fn upper_capabilities(&self) -> Option<&UpperFilesystemCapabilities> {
        self.upper_capabilities.as_ref()
    }

    pub(in overlayfs) fn whiteout_capability(&self) -> WhiteoutCapability {
        self.upper_capabilities()
            .map_or(WhiteoutCapability::Unsupported, |capabilities| {
                capabilities.whiteout_capability()
            })
    }

    pub(in overlayfs) fn can_store_private_xattr(&self) -> bool {
        self.upper_capabilities
            .is_some_and(|caps| caps.can_store_private_xattr())
    }

    pub(in overlayfs) fn can_express_whiteout(&self) -> bool {
        self.whiteout_capability() != WhiteoutCapability::Unsupported
    }
}
