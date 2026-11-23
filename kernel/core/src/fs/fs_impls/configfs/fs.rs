use alloc::sync::Arc;

use aster_systree::SysNode;
use spin::Once;

use super::inode::ConfigInode;
use crate::fs::{
    Result,
    configfs::systree_node::ConfigRootNode,
    pseudofs::AnonDeviceId,
    utils::systree_inode::SysTreeInodeTy,
    vfs::{
        file_system::{FileSystem, FsEventSubscriberStats, SuperBlock},
        inode::Inode,
        registry::{FsCreationCtx, FsProperties, FsType},
    },
};

pub(crate) struct ConfigFs {
    _anon_device_id: AnonDeviceId,
    sb: SuperBlock,
    root: Arc<dyn Inode>,
    fs_event_subscriber_stats: FsEventSubscriberStats,
}

const MAGIC_NUMBER: u64 = 0x62656570;
const BLOCK_SIZE: usize = 4096;
const NAME_MAX: usize = 255;

impl ConfigFs {
    pub(super) fn singleton() -> &'static Arc<ConfigFs> {
        static SINGLETON: Once<Arc<ConfigFs>> = Once::new();

        SINGLETON.call_once(|| Self::new(ConfigRootNode::singleton().clone()))
    }

    fn new(root_node: Arc<ConfigRootNode>) -> Arc<Self> {
        let anon_device_id =
            AnonDeviceId::acquire().expect("no device ID is available for configfs");
        let sb = SuperBlock::new(MAGIC_NUMBER, BLOCK_SIZE, NAME_MAX, anon_device_id.id());
        let root_inode = ConfigInode::new_root(root_node, &sb);

        Arc::new(Self {
            _anon_device_id: anon_device_id,
            sb,
            root: root_inode,
            fs_event_subscriber_stats: FsEventSubscriberStats::new(),
        })
    }
}

impl FileSystem for ConfigFs {
    fn name(&self) -> &'static str {
        "configfs"
    }

    fn sync(&self) -> Result<()> {
        Ok(())
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        self.root.clone()
    }

    fn sb(&self) -> SuperBlock {
        self.sb.clone()
    }

    fn max_name_len(&self) -> usize {
        self.sb.namelen
    }

    fn fs_event_subscriber_stats(&self) -> &FsEventSubscriberStats {
        &self.fs_event_subscriber_stats
    }
}

pub(super) struct ConfigFsType;

impl FsType for ConfigFsType {
    type Key = ();

    fn name(&self) -> &'static str {
        "configfs"
    }

    fn properties(&self) -> FsProperties {
        FsProperties::empty()
    }

    fn create(&self, _fs_creation_ctx: &mut FsCreationCtx) -> Result<Arc<dyn FileSystem>> {
        Ok(ConfigFs::singleton().clone())
    }

    fn sysnode(&self) -> Option<Arc<dyn SysNode>> {
        None
    }
}
