pub mod gpm_space;
mod guest_mode;

pub use self::{
    gpm_space::GuestPhysMemSpace,
    guest_mode::{DummyGuestHooks, GuestMode, GuestModeHooks, GuestReturnReason},
};

pub type Gpaddr = usize;
