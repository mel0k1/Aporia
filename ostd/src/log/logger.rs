use core::{
    fmt,
    sync::atomic::{AtomicU8, Ordering},
};

use spin::Once;

use super::{Level, LevelFilter, bridge::sync_log_crate_max_level};

static LOGGER: Once<&'static dyn Log> = Once::new();

pub fn inject_logger(logger: &'static dyn Log) {
    LOGGER.call_once(|| logger);
}

#[inline]
pub(super) fn __logger() -> Option<&'static dyn Log> {
    LOGGER.get().copied()
}

#[doc(hidden)]
pub fn __write_log_record(record: &Record) {
    if let Some(logger) = __logger() {
        logger.log(record);
    } else {
        crate::console::early_print(format_args!(
            "{}: {}{}\n",
            record.level(),
            record.prefix(),
            record.args()
        ));
    }
}

pub trait Log: Sync + Send {
    fn log(&self, record: &Record);
}

pub struct Record<'a> {
    level: Level,
    prefix: &'static str,
    args: fmt::Arguments<'a>,
    module_path: &'static str,
    file: &'static str,
    line: u32,
}

impl<'a> Record<'a> {
    #[doc(hidden)]
    #[inline]
    pub fn new(
        level: Level,
        prefix: &'static str,
        args: fmt::Arguments<'a>,
        module_path: &'static str,
        file: &'static str,
        line: u32,
    ) -> Self {
        Self {
            level,
            prefix,
            args,
            module_path,
            file,
            line,
        }
    }

    pub fn level(&self) -> Level {
        self.level
    }

    pub fn prefix(&self) -> &'static str {
        self.prefix
    }

    pub fn args(&self) -> &fmt::Arguments<'a> {
        &self.args
    }

    pub fn module_path(&self) -> &'static str {
        self.module_path
    }

    pub fn file(&self) -> &'static str {
        self.file
    }

    pub fn line(&self) -> u32 {
        self.line
    }
}

pub const STATIC_MAX_LEVEL: LevelFilter = LevelFilter::Debug;

static DYNAMIC_MAX_LEVEL: AtomicU8 = { AtomicU8::new(LevelFilter::Off as u8) };

pub fn set_max_level(mut filter: LevelFilter) {
    if filter > STATIC_MAX_LEVEL {
        filter = STATIC_MAX_LEVEL;
    }

    DYNAMIC_MAX_LEVEL.store(filter as u8, Ordering::Relaxed);
    sync_log_crate_max_level(filter);
}

#[inline]
pub fn max_level() -> LevelFilter {
    LevelFilter::from_u8(DYNAMIC_MAX_LEVEL.load(Ordering::Relaxed))
}
