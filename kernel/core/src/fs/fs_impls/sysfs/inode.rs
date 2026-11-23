use super::fs::SysFs;
use crate::{
    fs::{
        file::{InodeMode, InodeType},
        utils::systree_inode::{SysTreeInodeTy, SysTreeNodeKind},
        vfs::{
            file_system::FileSystem,
            inode::{Extension, Inode, Metadata, RevalidationPolicy},
            path::Dentry,
        },
    },
    prelude::*,
};

pub(super) struct SysFsInode {
    node_kind: SysTreeNodeKind,
    metadata: Metadata,
    extension: Extension,
    mode: RwLock<InodeMode>,
    parent: Weak<SysFsInode>,
    this: Weak<SysFsInode>,
}

impl SysTreeInodeTy for SysFsInode {
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

    fn extension(&self) -> &Extension {
        &self.extension
    }
}

impl Inode for SysFsInode {
    fn fs(&self) -> Arc<dyn FileSystem> {
        SysFs::singleton().clone()
    }

    fn create(
        &self,
        _self_dentry: &Dentry,
        _name: &str,
        _type_: InodeType,
        _mode: InodeMode,
    ) -> Result<Arc<dyn Inode>> {
        Err(errno!(EPERM, "file creation under sysfs is not allowed",))
    }

    fn revalidation_policy(&self) -> RevalidationPolicy {
        match self.node_kind() {
            SysTreeNodeKind::Branch(_) | SysTreeNodeKind::Leaf(_) => {
                RevalidationPolicy::REVALIDATE_EXISTS | RevalidationPolicy::REVALIDATE_ABSENT
            }
            SysTreeNodeKind::Attr(..) | SysTreeNodeKind::Symlink(_) => RevalidationPolicy::empty(),
        }
    }

    fn revalidate_exists(&self, name: &str, child: &dyn Inode) -> bool {
        let child = child.downcast_ref::<Self>().unwrap();

        let child_node_id = match child.node_kind() {
            SysTreeNodeKind::Attr(attr, node) => {
                return node.node_attrs().get(attr.name()).is_some_and(|current| {
                    current.id() == attr.id() && current.perms() == attr.perms()
                }) && !node.is_attr_absent(name);
            }
            SysTreeNodeKind::Branch(node) => *node.id(),
            SysTreeNodeKind::Leaf(node) => *node.id(),
            SysTreeNodeKind::Symlink(node) => *node.id(),
        };

        match self.node_kind() {
            SysTreeNodeKind::Branch(branch) => branch
                .child(name)
                .is_some_and(|current| *current.id() == child_node_id),
            _ => false,
        }
    }

    fn revalidate_absent(&self, _name: &str) -> bool {
        false
    }
}
