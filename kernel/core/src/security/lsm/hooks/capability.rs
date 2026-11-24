use super::super::modules;
use crate::{
    prelude::*,
    process::{UserNamespace, credentials::capabilities::CapSet, posix_thread::PosixThread},
};

pub fn on_capable(context: CapableContext) -> Result<()> {
    for module in modules::active_modules() {
        module.on_capable(&context)?;
    }

    Ok(())
}

pub struct CapableContext<'a> {
    target_user_ns: &'a UserNamespace,
    posix_thread: &'a PosixThread,
    required_cap: CapSet,
}

impl<'a> CapableContext<'a> {
    pub const fn new(
        target_user_ns: &'a UserNamespace,
        posix_thread: &'a PosixThread,
        required_cap: CapSet,
    ) -> Self {
        Self {
            target_user_ns,
            posix_thread,
            required_cap,
        }
    }

    #[expect(
        dead_code,
        reason = "Aporia currently has only the initial user namespace"
    )]
    pub(crate) const fn target_user_ns(&self) -> &UserNamespace {
        self.target_user_ns
    }

    pub(crate) const fn posix_thread(&self) -> &PosixThread {
        self.posix_thread
    }

    pub(crate) const fn required_cap(&self) -> CapSet {
        self.required_cap
    }
}
