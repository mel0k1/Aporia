use crate::{events::IoEvents, fs::file::file_table::FileDesc, prelude::*};

mod entry;
mod file;

pub(crate) use file::EpollFile;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum EpollCtl {
    Add(FileDesc, EpollEvent, EpollFlags),
    Del(FileDesc),
    Mod(FileDesc, EpollEvent, EpollFlags),
}

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub(crate) struct EpollFlags: u32 {
        const EXCLUSIVE    = (1 << 28);
        const WAKE_UP      = (1 << 29);
        const ONE_SHOT     = (1 << 30);
        const EDGE_TRIGGER = (1 << 31);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct EpollEvent {
    pub events: IoEvents,
    pub user_data: u64,
}

impl EpollEvent {
    pub(crate) fn new(events: IoEvents, user_data: u64) -> Self {
        Self { events, user_data }
    }
}
