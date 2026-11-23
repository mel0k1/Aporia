use aster_util::fixed_str::FixedCStr;
use ostd::sync::RwMutexReadGuard;
use spin::Once;

use crate::{
    fs::pseudofs::{NsCommonOps, NsType, StashedDentry},
    prelude::*,
    process::{UserNamespace, credentials::capabilities::CapSet, posix_thread::PosixThread},
    security::lsm::hooks as lsm_hooks,
};

pub(crate) struct UtsNamespace {
    uts_name: RwMutex<UtsName>,
    owner: Arc<UserNamespace>,
    stashed_dentry: StashedDentry,
}

impl UtsNamespace {
    pub(crate) fn get_init_singleton() -> &'static Arc<UtsNamespace> {
        static INIT: Once<Arc<UtsNamespace>> = Once::new();

        INIT.call_once(|| {
            let uts_name = UtsName {
                sysname: UtsField::from_bytes_until_nul(UtsName::SYSNAME.as_bytes()),
                nodename: UtsField::from_bytes_until_nul(b"(none)"),
                release: UtsField::from_bytes_until_nul(UtsName::RELEASE.as_bytes()),
                version: UtsField::from_bytes_until_nul(UtsName::VERSION.as_bytes()),
                machine: UtsField::from_bytes_until_nul(UtsName::MACHINE.as_bytes()),
                domainname: UtsField::from_bytes_until_nul(b"(none)"),
            };

            let owner = UserNamespace::get_init_singleton().clone();
            Self::new(uts_name, owner)
        })
    }

    fn new(uts_name: UtsName, owner: Arc<UserNamespace>) -> Arc<Self> {
        let stashed_dentry = StashedDentry::new();
        Arc::new(Self {
            uts_name: RwMutex::new(uts_name),
            owner,
            stashed_dentry,
        })
    }

    pub(crate) fn new_clone(
        &self,
        owner: Arc<UserNamespace>,
        posix_thread: &PosixThread,
    ) -> Result<Arc<Self>> {
        lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
            owner.as_ref(),
            posix_thread,
            CapSet::SYS_ADMIN,
        ))?;
        Ok(Self::new(*self.uts_name.read(), owner))
    }

    pub(crate) fn uts_name(&self) -> RwMutexReadGuard<'_, UtsName> {
        self.uts_name.read()
    }

    pub(crate) fn set_hostname(
        &self,
        new_host_name: UtsField,
        posix_thread: &PosixThread,
    ) -> Result<()> {
        self.check_set_permission(posix_thread)?;
        self.set_hostname_field(new_host_name);
        Ok(())
    }

    pub(crate) fn set_domainname(
        &self,
        new_domain_name: UtsField,
        posix_thread: &PosixThread,
    ) -> Result<()> {
        self.check_set_permission(posix_thread)?;
        self.set_domainname_field(new_domain_name);
        Ok(())
    }

    fn check_set_permission(&self, posix_thread: &PosixThread) -> Result<()> {
        lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
            self.owner.as_ref(),
            posix_thread,
            CapSet::SYS_ADMIN,
        ))
    }

    fn set_hostname_field(&self, new_host_name: UtsField) {
        debug!("set host name: {:?}", new_host_name.as_cstr());
        self.uts_name.write().nodename = new_host_name;
    }

    fn set_domainname_field(&self, new_domain_name: UtsField) {
        debug!("set domain name: {:?}", new_domain_name.as_cstr());
        self.uts_name.write().domainname = new_domain_name;
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct UtsName {
    sysname: UtsField,
    nodename: UtsField,
    release: UtsField,
    version: UtsField,
    machine: UtsField,
    domainname: UtsField,
}

impl UtsName {
    pub(crate) const SYSNAME: &str = "Linux";

    pub(crate) const RELEASE: &str = "5.13.0";

    pub(crate) const VERSION: &str = {
        const BUILD_TIMESTAMP: &str = if let Some(timestamp) = option_env!("ASTER_BUILD_TIMESTAMP")
        {
            timestamp
        } else {
            const UNIX_EPOCH: &str = "Thu Jan  1 00:00:00 UTC 1970";
            UNIX_EPOCH
        };

        const BUILD_VERSION: usize = 1;
        const SMP_FLAGS: &str = "SMP ";
        const PREEMPT_FLAGS: &str = "";
        const VERSION: &str =
            const_format::formatcp!("#{BUILD_VERSION} {SMP_FLAGS}{PREEMPT_FLAGS}{BUILD_TIMESTAMP}");
        assert!(VERSION.len() <= UtsField::MAX_BYTES);
        VERSION
    };

    pub(crate) const MACHINE: &str = cfg_select! {
        target_arch = "x86_64" => "x86_64",
        target_arch = "riscv64" => "riscv64",
        target_arch = "loongarch64" => "loongarch64",
        target_arch = "aarch64" => "aarch64",
        _ => compile_error!("unsupported target"),
    };

    pub(crate) fn nodename(&self) -> &UtsField {
        &self.nodename
    }

    pub(crate) fn domainname(&self) -> &UtsField {
        &self.domainname
    }
}

const UTS_FIELD_LEN: usize = 65;

pub(crate) type UtsField = FixedCStr<UTS_FIELD_LEN>;

impl NsCommonOps for UtsNamespace {
    const TYPE: NsType = NsType::Uts;

    fn owner_user_ns(&self) -> Option<&Arc<UserNamespace>> {
        Some(&self.owner)
    }

    fn parent(&self) -> Result<&Arc<Self>> {
        Err(errno!(
            EINVAL,
            "a UTS namespace does not have a parent namespace"
        ))
    }

    fn stashed_dentry(&self) -> &StashedDentry {
        &self.stashed_dentry
    }
}
