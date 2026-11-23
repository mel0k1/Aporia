#[macro_export]
macro_rules! emerg {
    ($($arg:tt)+) => { $crate::log!($crate::log::Level::Emerg, $($arg)+) };
}

#[macro_export]
macro_rules! alert {
    ($($arg:tt)+) => { $crate::log!($crate::log::Level::Alert, $($arg)+) };
}

#[macro_export]
macro_rules! crit {
    ($($arg:tt)+) => { $crate::log!($crate::log::Level::Crit, $($arg)+) };
}

#[macro_export]
macro_rules! error {
    ($($arg:tt)+) => { $crate::log!($crate::log::Level::Error, $($arg)+) };
}

#[macro_export]
macro_rules! warn {
    ($($arg:tt)+) => { $crate::log!($crate::log::Level::Warning, $($arg)+) };
}

#[macro_export]
macro_rules! notice {
    ($($arg:tt)+) => { $crate::log!($crate::log::Level::Notice, $($arg)+) };
}

#[macro_export]
macro_rules! info {
    ($($arg:tt)+) => { $crate::log!($crate::log::Level::Info, $($arg)+) };
}

#[macro_export]
macro_rules! debug {
    ($($arg:tt)+) => { $crate::log!($crate::log::Level::Debug, $($arg)+) };
}

#[macro_export]
macro_rules! log_enabled {
    ($level:expr) => {{
        let __level: $crate::log::Level = $level;
        $crate::log::STATIC_MAX_LEVEL.is_enabled(__level)
            && $crate::log::max_level().is_enabled(__level)
    }};
}

#[macro_export]
macro_rules! log {
    ($level:expr, $($arg:tt)+) => {{
        const __LEVEL: $crate::log::Level = $level;
        if $crate::log_enabled!(__LEVEL) {
            $crate::log::__write_log_record(&$crate::log::Record::new(
                __LEVEL,
                __log_prefix!(),
                format_args!($($arg)+),
                module_path!(),
                file!(),
                line!(),
            ));
        }
    }};
}

mod about_log_prefix_macro {}
