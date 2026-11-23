pub(crate) mod fault;
pub(crate) mod kernel;
pub(crate) mod raw;
pub(crate) mod timer;
pub(crate) mod user;

use core::{any::Any, fmt::Debug};

use super::{c_types::siginfo_t, sig_num::SigNum};

pub(crate) trait Signal: Send + Sync + Debug + Any {
    fn num(&self) -> SigNum;

    fn to_info(&self) -> siginfo_t;
}
