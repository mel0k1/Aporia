use crate::{prelude::*, process::posix_thread::PosixThread, security::lsm::hooks as lsm_hooks};

impl PosixThread {
    pub(crate) fn check_alien_access_from(
        &self,
        accessor: &PosixThread,
        mode: AlienAccessMode,
    ) -> Result<()> {
        if Weak::ptr_eq(accessor.weak_process(), self.weak_process()) {
            return Ok(());
        }

        lsm_hooks::on_alien_access(lsm_hooks::AlienAccessContext::new(accessor, self, mode))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CredsSource {
    FsCreds,
    RealCreds,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AlienAccessKind {
    Read,
    Attach,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AlienAccessMode {
    kind: AlienAccessKind,
    creds: CredsSource,
}

impl AlienAccessMode {
    #[expect(dead_code)]
    pub(crate) const READ_WITH_REAL_CREDS: Self =
        Self::new(AlienAccessKind::Read, CredsSource::RealCreds);
    pub(crate) const ATTACH_WITH_REAL_CREDS: Self =
        Self::new(AlienAccessKind::Attach, CredsSource::RealCreds);
    pub(crate) const READ_WITH_FS_CREDS: Self =
        Self::new(AlienAccessKind::Read, CredsSource::FsCreds);
    pub(crate) const ATTACH_WITH_FS_CREDS: Self =
        Self::new(AlienAccessKind::Attach, CredsSource::FsCreds);

    pub(crate) const fn new(kind: AlienAccessKind, creds: CredsSource) -> Self {
        Self { kind, creds }
    }

    pub(crate) const fn kind(self) -> AlienAccessKind {
        self.kind
    }

    pub(crate) const fn creds(self) -> CredsSource {
        self.creds
    }
}
