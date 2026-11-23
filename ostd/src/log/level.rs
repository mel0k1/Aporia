use core::fmt;

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Level {
    Emerg = 0,
    Alert = 1,
    Crit = 2,
    Error = 3,
    Warning = 4,
    Notice = 5,
    Info = 6,
    Debug = 7,
}

impl Level {
    pub const fn from_u8(val: u8) -> Self {
        match val {
            0 => Self::Emerg,
            1 => Self::Alert,
            2 => Self::Crit,
            3 => Self::Error,
            4 => Self::Warning,
            5 => Self::Notice,
            6 => Self::Info,
            _ => Self::Debug,
        }
    }
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(match self {
            Level::Emerg => "EMERG",
            Level::Alert => "ALERT",
            Level::Crit => "CRIT",
            Level::Error => "ERROR",
            Level::Warning => "WARN",
            Level::Notice => "NOTICE",
            Level::Info => "INFO",
            Level::Debug => "DEBUG",
        })
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum LevelFilter {
    Off = 0,
    Emerg = 1,
    Alert = 2,
    Crit = 3,
    Error = 4,
    Warning = 5,
    Notice = 6,
    Info = 7,
    Debug = 8,
}

impl LevelFilter {
    #[inline]
    pub const fn is_enabled(self, level: Level) -> bool {
        (self as u8) > (level as u8)
    }

    pub const fn from_level(level: Level) -> Self {
        match level {
            Level::Emerg => Self::Emerg,
            Level::Alert => Self::Alert,
            Level::Crit => Self::Crit,
            Level::Error => Self::Error,
            Level::Warning => Self::Warning,
            Level::Notice => Self::Notice,
            Level::Info => Self::Info,
            Level::Debug => Self::Debug,
        }
    }

    pub const fn from_u8(val: u8) -> Self {
        match val {
            0 => Self::Off,
            1 => Self::Emerg,
            2 => Self::Alert,
            3 => Self::Crit,
            4 => Self::Error,
            5 => Self::Warning,
            6 => Self::Notice,
            7 => Self::Info,
            _ => Self::Debug,
        }
    }
}

#[cfg(ktest)]
mod test {
    use super::*;
    use crate::prelude::*;

    #[ktest]
    fn level_ordering() {
        assert!(Level::Emerg < Level::Alert);
        assert!(Level::Alert < Level::Crit);
        assert!(Level::Crit < Level::Error);
        assert!(Level::Error < Level::Warning);
        assert!(Level::Warning < Level::Notice);
        assert!(Level::Notice < Level::Info);
        assert!(Level::Info < Level::Debug);
    }

    #[ktest]
    fn level_filter_enabled() {
        assert!(!LevelFilter::Off.is_enabled(Level::Emerg));
        assert!(LevelFilter::Emerg.is_enabled(Level::Emerg));
        assert!(!LevelFilter::Emerg.is_enabled(Level::Alert));
        assert!(LevelFilter::Error.is_enabled(Level::Error));
        assert!(LevelFilter::Error.is_enabled(Level::Crit));
        assert!(!LevelFilter::Error.is_enabled(Level::Warning));
        assert!(LevelFilter::Debug.is_enabled(Level::Debug));
    }

    #[ktest]
    fn level_from_u8_clamping() {
        assert_eq!(Level::from_u8(0), Level::Emerg);
        assert_eq!(Level::from_u8(7), Level::Debug);
        assert_eq!(Level::from_u8(255), Level::Debug);
    }

    #[ktest]
    fn level_filter_from_level() {
        assert_eq!(LevelFilter::from_level(Level::Error), LevelFilter::Error);
        assert_eq!(LevelFilter::from_level(Level::Debug), LevelFilter::Debug);
    }
}
