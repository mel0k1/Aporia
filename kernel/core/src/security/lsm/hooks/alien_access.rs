use super::super::modules;
use crate::{
    prelude::*,
    process::posix_thread::{PosixThread, alien_access::AlienAccessMode},
};

pub(crate) fn on_alien_access(context: AlienAccessContext) -> Result<()> {
    for module in modules::active_modules() {
        module.on_alien_access(&context)?;
    }

    Ok(())
}

pub(crate) struct AlienAccessContext<'a> {
    accessor: &'a PosixThread,
    target: &'a PosixThread,
    mode: AlienAccessMode,
}

impl<'a> AlienAccessContext<'a> {
    pub(crate) const fn new(
        accessor: &'a PosixThread,
        target: &'a PosixThread,
        mode: AlienAccessMode,
    ) -> Self {
        Self {
            accessor,
            target,
            mode,
        }
    }

    pub(crate) const fn accessor(&self) -> &PosixThread {
        self.accessor
    }

    pub(crate) const fn target(&self) -> &PosixThread {
        self.target
    }

    pub(crate) const fn mode(&self) -> AlienAccessMode {
        self.mode
    }
}
