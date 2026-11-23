use spin::Once;

use crate::{
    fs::pseudofs::{NsCommonOps, NsType, StashedDentry},
    prelude::*,
    process::Uid,
};

pub struct UserNamespace {
    stashed_dentry: StashedDentry,
}

impl UserNamespace {
    pub fn get_init_singleton() -> &'static Arc<UserNamespace> {
        static INIT: Once<Arc<UserNamespace>> = Once::new();

        INIT.call_once(|| {
            Arc::new(Self {
                stashed_dentry: StashedDentry::new(),
            })
        })
    }

    pub(crate) fn owner_uid(&self) -> Result<Uid> {
        Ok(Uid::new_root())
    }

    pub(crate) fn is_same_or_ancestor_of(self: &Arc<Self>, other: &Arc<Self>) -> bool {
        Arc::ptr_eq(self, other)
    }
}

impl NsCommonOps for UserNamespace {
    const TYPE: NsType = NsType::User;

    fn owner_user_ns(&self) -> Option<&Arc<UserNamespace>> {
        None
    }

    fn parent(&self) -> Result<&Arc<Self>> {
        Err(errno!(
            EPERM,
            "user namespaces do not support NS_GET_PARENT"
        ))
    }

    fn stashed_dentry(&self) -> &StashedDentry {
        &self.stashed_dentry
    }
}
