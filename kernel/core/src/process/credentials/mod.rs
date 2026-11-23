pub(crate) mod c_types;
pub mod capabilities;
mod credentials_;
mod file_capabilities;
mod group;
mod secure_bits;
mod static_cap;
mod user;

use aster_rights::FullOp;
use credentials_::Credentials_;
pub(super) use credentials_::ExecCred;
pub(crate) use file_capabilities::{FileCapabilities, VfsCapRevision};
pub(crate) use group::Gid;
pub(crate) use secure_bits::SecureBits;
pub(crate) use user::Uid;

use crate::prelude::*;

pub(crate) struct Credentials<R = FullOp>(Arc<Credentials_>, R);
