use ostd::{boot::EarlyCmdline, log::LevelFilter};

const EARLY_PARAMS: &[&str] = &["earlycon", "loglevel"];

pub(super) fn is_early_param(key: &str) -> bool {
    EARLY_PARAMS.contains(&key)
}

#[ostd::early_cmdline_parser]
const fn early_cmdline_parser(cmdline: &str) -> EarlyCmdline {
    let mut has_early_console = false;
    let mut log_level = LevelFilter::Debug;

    let bytes: &[u8] = cmdline.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() {
            break;
        }

        let start: usize = index;
        let mut inside_quotes = false;
        while index < bytes.len() {
            let byte = bytes[index];
            if byte == b'"' {
                inside_quotes = !inside_quotes;
            } else if byte.is_ascii_whitespace() && !inside_quotes {
                break;
            }
            index += 1;
        }

        let token = strip_linux_double_quotes(bytes, start, index);

        if matches!(token, b"--") {
            break;
        } else if matches!(token, b"earlycon") {
            has_early_console = true;
        } else if let Some((key, value)) = token.split_at_checked(9)
            && matches!(key, b"loglevel=")
            && let Some(parsed_log_level) = parse_loglevel_at(value)
        {
            log_level = parsed_log_level;
        }
    }

    EarlyCmdline {
        log_level,
        has_early_console,
    }
}

const fn strip_linux_double_quotes(bytes: &[u8], start: usize, end: usize) -> &[u8] {
    let (new_start, new_end) = if start < end && bytes[start] == b'"' {
        let new_start = start + 1;
        if new_start < end && bytes[end - 1] == b'"' {
            (new_start, end - 1)
        } else {
            (new_start, end)
        }
    } else {
        (start, end)
    };
    bytes.split_at(new_start).1.split_at(new_end - new_start).0
}

const fn parse_loglevel_at(bytes: &[u8]) -> Option<LevelFilter> {
    let bytes = strip_linux_double_quotes(bytes, 0, bytes.len());

    if bytes.is_empty() {
        return None;
    }

    if bytes[0].is_ascii_digit() {
        if let Ok(result) = u8::from_ascii_bytes_radix(bytes, 10)
            && result <= 8
        {
            return Some(LevelFilter::from_u8(result));
        }
        return None;
    }

    parse_loglevel_name_at(bytes)
}

const fn parse_loglevel_name_at(bytes: &[u8]) -> Option<LevelFilter> {
    match bytes {
        b"off" => Some(LevelFilter::Off),
        b"emerg" => Some(LevelFilter::Emerg),
        b"alert" => Some(LevelFilter::Alert),
        b"crit" => Some(LevelFilter::Crit),
        b"error" | b"err" => Some(LevelFilter::Error),
        b"warning" | b"warn" => Some(LevelFilter::Warning),
        b"notice" => Some(LevelFilter::Notice),
        b"info" => Some(LevelFilter::Info),
        b"debug" => Some(LevelFilter::Debug),
        _ => None,
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::*;

    use super::*;

    #[ktest]
    fn is_early_param_recognizes_early_keys() {
        assert!(is_early_param("earlycon"));
        assert!(is_early_param("loglevel"));
        assert!(!is_early_param("log_level"));
        assert!(!is_early_param("unknown"));
    }

    #[ktest]
    fn early_cmdline_parser_defaults() {
        let result = early_cmdline_parser("");
        assert_eq!(result.log_level, LevelFilter::Debug);
        assert!(!result.has_early_console);
    }

    #[ktest]
    fn early_cmdline_parser_earlycon() {
        assert!(early_cmdline_parser("earlycon").has_early_console);
        assert!(early_cmdline_parser("\"earlycon\"").has_early_console);
        assert!(!early_cmdline_parser("\"  earlycon \"").has_early_console);
        assert!(!early_cmdline_parser("earlyconxxxx").has_early_console);
    }

    #[ktest]
    fn early_cmdline_parser_loglevel() {
        assert_eq!(
            early_cmdline_parser("loglevel=0").log_level,
            LevelFilter::Off
        );
        assert_eq!(
            early_cmdline_parser("loglevel=\"0\"").log_level,
            LevelFilter::Off
        );
        assert_eq!(
            early_cmdline_parser("\"loglevel=0\"").log_level,
            LevelFilter::Off
        );
        assert_eq!(
            early_cmdline_parser("loglevel=3").log_level,
            LevelFilter::Crit
        );
        assert_eq!(
            early_cmdline_parser("loglevel=3x").log_level,
            LevelFilter::Debug
        );
        assert_eq!(
            early_cmdline_parser("loglevel=9").log_level,
            LevelFilter::Debug
        );
        assert_eq!(
            early_cmdline_parser("loglevel=error").log_level,
            LevelFilter::Error
        );
        assert_eq!(
            early_cmdline_parser("loglevel=err").log_level,
            LevelFilter::Error
        );
        assert_eq!(
            early_cmdline_parser("loglevel=off").log_level,
            LevelFilter::Off
        );
        assert_eq!(
            early_cmdline_parser("loglevel=warn").log_level,
            LevelFilter::Warning
        );
        assert_eq!(
            early_cmdline_parser("loglevel=Warn").log_level,
            LevelFilter::Debug
        );
    }

    #[ktest]
    fn early_cmdline_parser_last_loglevel_wins() {
        assert_eq!(
            early_cmdline_parser("loglevel=4 loglevel=0").log_level,
            LevelFilter::Off
        );
    }

    #[ktest]
    fn early_cmdline_parser_invalid_loglevel_does_not_override() {
        assert_eq!(
            early_cmdline_parser("loglevel=0 loglevel=3x").log_level,
            LevelFilter::Off
        );
        assert_eq!(
            early_cmdline_parser("loglevel=error loglevel=").log_level,
            LevelFilter::Error
        );
        assert_eq!(
            early_cmdline_parser("loglevel=warn loglevel=9").log_level,
            LevelFilter::Warning
        );
    }

    #[ktest]
    fn early_cmdline_parser_stops_at_double_dash() {
        let result = early_cmdline_parser("loglevel=0 -- loglevel=4 earlycon");
        assert_eq!(result.log_level, LevelFilter::Off);
        assert!(!result.has_early_console);
    }

    #[ktest]
    fn early_cmdline_parser_quotes_protect_spaces() {
        let result = early_cmdline_parser("loglevel=0 \"ignored -- token\" earlycon");
        assert_eq!(result.log_level, LevelFilter::Off);
        assert!(result.has_early_console);
    }
}
