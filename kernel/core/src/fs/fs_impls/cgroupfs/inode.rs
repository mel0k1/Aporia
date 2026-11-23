use super::fs::CgroupFs;
use crate::{
    fs::{
        cgroupfs::CgroupNode,
        file::InodeMode,
        utils::systree_inode::{SysTreeInodeTy, SysTreeNodeKind},
        vfs::{
            file_system::FileSystem,
            inode::{Extension, Inode, Metadata, RevalidationPolicy},
            path::Dentry,
        },
    },
    prelude::*,
};

pub(super) struct CgroupInode {
    node_kind: SysTreeNodeKind,
    metadata: Metadata,
    extension: Extension,
    mode: RwLock<InodeMode>,
    parent: Weak<CgroupInode>,
    this: Weak<CgroupInode>,
}

impl SysTreeInodeTy for CgroupInode {
    fn new_arc(
        node_kind: SysTreeNodeKind,
        metadata: Metadata,
        mode: InodeMode,
        parent: Weak<Self>,
    ) -> Arc<Self>
    where
        Self: Sized,
    {
        Arc::new_cyclic(|this| Self {
            node_kind,
            metadata,
            extension: Extension::new(),
            mode: RwLock::new(mode),
            parent,
            this: this.clone(),
        })
    }

    fn node_kind(&self) -> &SysTreeNodeKind {
        &self.node_kind
    }

    fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    fn extension(&self) -> &Extension {
        &self.extension
    }

    fn mode(&self) -> Result<InodeMode> {
        Ok(*self.mode.read())
    }

    fn set_mode(&self, mode: InodeMode) -> Result<()> {
        *self.mode.write() = mode;
        Ok(())
    }

    fn parent(&self) -> &Weak<Self> {
        &self.parent
    }

    fn this(&self) -> Arc<Self> {
        self.this
            .upgrade()
            .expect("invalid weak reference to `self`")
    }
}

impl Inode for CgroupInode {
    fn fs(&self) -> Arc<dyn FileSystem> {
        CgroupFs::singleton().clone()
    }

    fn rmdir(&self, child_dentry: &Dentry) -> Result<()> {
        let name = child_dentry.name();
        let SysTreeNodeKind::Branch(branch_node) = self.node_kind() else {
            return Err(errno!(ENOTDIR, "the current node is not a branch node"));
        };

        let Some(child) = branch_node.child(&name) else {
            return Err(errno!(ENOENT, "the child node does not exist"));
        };

        let target_node = Arc::downcast::<CgroupNode>(child).unwrap();

        target_node.mark_as_dead()?;

        branch_node.remove_child(&name).unwrap();

        Ok(())
    }

    fn revalidation_policy(&self) -> RevalidationPolicy {
        match self.node_kind() {
            SysTreeNodeKind::Branch(_) | SysTreeNodeKind::Leaf(_) => {
                RevalidationPolicy::REVALIDATE_EXISTS | RevalidationPolicy::REVALIDATE_ABSENT
            }
            SysTreeNodeKind::Attr(..) | SysTreeNodeKind::Symlink(_) => RevalidationPolicy::empty(),
        }
    }

    fn revalidate_exists(&self, _name: &str, child: &dyn Inode) -> bool {
        child
            .downcast_ref::<Self>()
            .is_some_and(|child| !matches!(child.node_kind(), SysTreeNodeKind::Attr(..)))
    }

    fn revalidate_absent(&self, _name: &str) -> bool {
        false
    }
}
