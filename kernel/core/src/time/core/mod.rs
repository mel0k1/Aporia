use core::time::Duration;

pub(crate) mod timer;

pub(crate) trait Clock: Send + Sync {
    fn read_time(&self) -> Duration;
}
