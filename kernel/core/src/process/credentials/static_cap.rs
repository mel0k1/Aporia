use aster_rights::{Dup, Read, TRights, Write};
use aster_rights_proc::require;
use ostd::sync::{PreemptDisabled, RwLockReadGuard, RwLockWriteGuard};

use super::{
    Credentials, ExecCred, FileCapabilities, Gid, SecureBits, Uid, capabilities::CapSet,
    credentials_::Credentials_,
};
use crate::prelude::*;

impl<R: TRights> Credentials<R> {
    pub(crate) fn new_root() -> Self {
        let uid = Uid::new_root();
        let gid = Gid::new_root();
        let cap = CapSet::new_root();
        let credentials_ = Arc::new(Credentials_::new(uid, gid, cap));
        Self(credentials_, R::new())
    }

    #[require(R1 > Read)]
    pub(crate) fn new_from<R1: TRights>(credentials: &Credentials<R1>) -> Self {
        let credentials_ = Arc::new(credentials.0.as_ref().clone());

        Self(credentials_, R::new())
    }

    #[require(R > Dup)]
    pub(crate) fn dup(&self) -> Self {
        Self(self.0.clone(), self.1)
    }

    #[require(R > R1)]
    pub(crate) fn restrict<R1: TRights>(self) -> Credentials<R1> {
        let Credentials(credentials_, _) = self;

        Credentials(credentials_, R1::new())
    }

    #[require(R > Read)]
    pub(crate) fn ruid(&self) -> Uid {
        self.0.ruid()
    }

    #[require(R > Read)]
    pub(crate) fn euid(&self) -> Uid {
        self.0.euid()
    }

    #[require(R > Read)]
    pub(crate) fn suid(&self) -> Uid {
        self.0.suid()
    }

    #[require(R > Read)]
    pub(crate) fn fsuid(&self) -> Uid {
        self.0.fsuid()
    }

    #[require(R > Write)]
    pub(crate) fn set_uid(&self, uid: Uid) -> Result<()> {
        self.0.set_uid(uid)
    }

    #[require(R > Write)]
    pub(crate) fn set_reuid(&self, ruid: Option<Uid>, euid: Option<Uid>) -> Result<()> {
        self.0.set_reuid(ruid, euid)
    }

    #[require(R > Write)]
    pub(crate) fn set_resuid(
        &self,
        ruid: Option<Uid>,
        euid: Option<Uid>,
        suid: Option<Uid>,
    ) -> Result<()> {
        self.0.set_resuid(ruid, euid, suid)
    }

    #[require(R > Write)]
    pub(crate) fn set_fsuid(&self, fsuid: Option<Uid>) -> Result<Uid, Uid> {
        self.0.set_fsuid(fsuid)
    }

    #[require(R > Read)]
    pub(crate) fn rgid(&self) -> Gid {
        self.0.rgid()
    }

    #[require(R > Read)]
    pub(crate) fn egid(&self) -> Gid {
        self.0.egid()
    }

    #[require(R > Read)]
    pub(crate) fn sgid(&self) -> Gid {
        self.0.sgid()
    }

    #[require(R > Read)]
    pub(crate) fn fsgid(&self) -> Gid {
        self.0.fsgid()
    }

    #[require(R > Write)]
    pub(crate) fn set_gid(&self, gid: Gid) -> Result<()> {
        self.0.set_gid(gid)
    }

    #[require(R > Write)]
    pub(crate) fn set_regid(&self, rgid: Option<Gid>, egid: Option<Gid>) -> Result<()> {
        self.0.set_regid(rgid, egid)
    }

    #[require(R > Write)]
    pub(crate) fn set_resgid(
        &self,
        rgid: Option<Gid>,
        egid: Option<Gid>,
        sgid: Option<Gid>,
    ) -> Result<()> {
        self.0.set_resgid(rgid, egid, sgid)
    }

    #[require(R > Write)]
    pub(crate) fn set_fsgid(&self, fsgid: Option<Gid>) -> Result<Gid, Gid> {
        self.0.set_fsgid(fsgid)
    }

    #[require(R > Read)]
    pub(crate) fn groups(&self) -> RwLockReadGuard<'_, BTreeSet<Gid>, PreemptDisabled> {
        self.0.groups()
    }

    #[require(R > Read)]
    pub(crate) fn has_group_for_fs_access(&self, gid: Gid) -> bool {
        self.fsgid() == gid || self.groups().contains(&gid)
    }

    #[require(R > Write)]
    pub(crate) fn groups_mut(&self) -> RwLockWriteGuard<'_, BTreeSet<Gid>, PreemptDisabled> {
        self.0.groups_mut()
    }

    #[require(R > Read)]
    pub(crate) fn inheritable_capset(&self) -> CapSet {
        self.0.inheritable_capset()
    }

    #[require(R > Read)]
    pub(crate) fn permitted_capset(&self) -> CapSet {
        self.0.permitted_capset()
    }

    #[require(R > Read)]
    pub(crate) fn effective_capset(&self) -> CapSet {
        self.0.effective_capset()
    }

    #[require(R > Read)]
    pub(crate) fn bounding_capset(&self) -> CapSet {
        self.0.bounding_capset()
    }

    #[require(R > Write)]
    pub(crate) fn set_inheritable_capset(&self, inheritable_capset: CapSet) {
        self.0.set_inheritable_capset(inheritable_capset);
    }

    #[require(R > Write)]
    pub(crate) fn set_permitted_capset(&self, permitted_capset: CapSet) {
        self.0.set_permitted_capset(permitted_capset);
    }

    #[require(R > Write)]
    pub(crate) fn set_effective_capset(&self, effective_capset: CapSet) {
        self.0.set_effective_capset(effective_capset);
    }

    #[require(R > Read)]
    pub(crate) fn ambient_capset(&self) -> CapSet {
        self.0.ambient_capset()
    }

    #[require(R > Write)]
    pub(crate) fn raise_ambient_capability(&self, capability: CapSet) -> Result<()> {
        self.0.raise_ambient_capability(capability)
    }

    #[require(R > Write)]
    pub(crate) fn lower_ambient_capability(&self, capability: CapSet) {
        self.0.lower_ambient_capability(capability);
    }

    #[require(R > Write)]
    pub(crate) fn clear_ambient_capset(&self) {
        self.0.clear_ambient_capset();
    }

    #[require(R > Write)]
    pub(crate) fn drop_bounding_capability(&self, capability: CapSet) -> Result<()> {
        self.0.drop_bounding_capability(capability)
    }

    #[require(R > Read)]
    pub(crate) fn keep_capabilities(&self) -> bool {
        self.0.keep_capabilities()
    }

    #[require(R > Write)]
    pub(crate) fn set_keep_capabilities(&self, keep_capabilities: bool) -> Result<()> {
        self.0.set_keep_capabilities(keep_capabilities)
    }

    #[require(R > Read)]
    pub(crate) fn securebits(&self) -> SecureBits {
        self.0.securebits()
    }

    #[require(R > Write)]
    pub(crate) fn set_securebits(&self, securebits: SecureBits) -> Result<()> {
        self.0.set_securebits(securebits)
    }

    #[require(R > Read)]
    pub(crate) fn no_new_privs(&self) -> bool {
        self.0.no_new_privs()
    }

    #[require(R > Write)]
    pub(crate) fn set_no_new_privs(&self) {
        self.0.set_no_new_privs();
    }

    #[require(R > Read)]
    pub(in crate::process) fn prepare_exec_cred(
        &self,
        file_capabilities: Option<FileCapabilities>,
        setuid: Option<Uid>,
        setgid: Option<Gid>,
    ) -> Result<ExecCred> {
        self.0.prepare_exec_cred(file_capabilities, setuid, setgid)
    }

    #[require(R > Write)]
    pub(in crate::process) fn apply_exec_cred(&self, exec_cred: ExecCred) -> Result<()> {
        self.0.apply_exec_cred(exec_cred)
    }
}
