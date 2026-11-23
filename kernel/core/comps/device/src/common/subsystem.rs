use alloc::sync::Arc;

use crate::common::{AnyDevice, node::Dir};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubsystemKind {
    Bus,
    Class,
    Bare,
}

#[derive(Clone)]
pub struct Subsystem {
    kind: SubsystemKind,
    ops: Option<Arc<dyn SubsystemOps>>,
}

impl Subsystem {
    pub fn kind(&self) -> SubsystemKind {
        self.kind
    }

    pub fn is_bus(&self) -> bool {
        matches!(self.kind, SubsystemKind::Bus)
    }

    pub fn is_class(&self) -> bool {
        matches!(self.kind, SubsystemKind::Class)
    }

    pub fn is_bare(&self) -> bool {
        matches!(self.kind, SubsystemKind::Bare)
    }

    pub fn name(&self) -> Option<&'static str> {
        self.ops.as_ref().map(|ops| ops.name())
    }

    pub(crate) fn bus(ops: Arc<dyn SubsystemOps>) -> Self {
        Self {
            kind: SubsystemKind::Bus,
            ops: Some(ops),
        }
    }

    pub(crate) fn class(ops: Arc<dyn SubsystemOps>) -> Self {
        Self {
            kind: SubsystemKind::Class,
            ops: Some(ops),
        }
    }

    pub(crate) const fn bare() -> Self {
        Self {
            kind: SubsystemKind::Bare,
            ops: None,
        }
    }

    pub(crate) fn ops(&self) -> Option<&Arc<dyn SubsystemOps>> {
        self.ops.as_ref()
    }

    pub(crate) fn dir(&self) -> Option<Arc<Dir>> {
        self.ops.as_ref().map(|ops| ops.dir())
    }

    pub(crate) fn index_dir(&self) -> Option<Arc<Dir>> {
        self.ops.as_ref().map(|ops| ops.index_dir())
    }

    pub(crate) fn keeps_glue_dir(&self) -> bool {
        self.ops.as_ref().is_some_and(|ops| ops.keeps_glue_dir())
    }
}

impl core::fmt::Debug for Subsystem {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Subsystem")
            .field("kind", &self.kind)
            .field("name", &self.name())
            .finish()
    }
}

pub(crate) trait SubsystemOps: Send + Sync + 'static {
    fn name(&self) -> &'static str;

    fn dir(&self) -> Arc<Dir>;

    fn index_dir(&self) -> Arc<Dir>;

    fn keeps_glue_dir(&self) -> bool;

    fn on_added(&self, dev: &Arc<dyn AnyDevice>);

    fn on_removed(&self, dev: &Arc<dyn AnyDevice>);
}
