use alloc::vec::Vec;
use core::{
    str::FromStr,
    sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering},
};

use spin::Once;

pub trait ParamStorage: Sync + 'static {
    type Value;
    fn store_param(&self, value: Self::Value);
}

impl<T: Send + Sync + 'static> ParamStorage for Once<T> {
    type Value = T;
    fn store_param(&self, value: T) {
        self.call_once(|| value);
    }
}

impl ParamStorage for AtomicU8 {
    type Value = u8;
    fn store_param(&self, value: u8) {
        self.store(value, Ordering::Relaxed);
    }
}

impl ParamStorage for AtomicU32 {
    type Value = u32;
    fn store_param(&self, value: u32) {
        self.store(value, Ordering::Relaxed);
    }
}

impl ParamStorage for AtomicBool {
    type Value = bool;
    fn store_param(&self, value: bool) {
        self.store(value, Ordering::Relaxed);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParamError {
    InvalidValue,
}

pub trait ParseParamValue: Sized {
    fn parse_param(value: &str) -> Result<Self, ParamError>;
}

pub trait ParseRepeatableParamValue: Sized {
    fn parse_all(values: &[&str]) -> Result<Self, ParamError>;
}

pub trait ParseFlag: Sized {
    fn parse_flag(value: Option<&str>) -> Result<Self, ParamError>;
}

impl<T: FromStr> ParseParamValue for T {
    fn parse_param(value: &str) -> Result<Self, ParamError> {
        value.parse().map_err(|_| ParamError::InvalidValue)
    }
}

impl<T: ParseParamValue> ParseRepeatableParamValue for Vec<T> {
    fn parse_all(values: &[&str]) -> Result<Self, ParamError> {
        values.iter().map(|value| T::parse_param(value)).collect()
    }
}

impl ParseFlag for bool {
    fn parse_flag(value: Option<&str>) -> Result<Self, ParamError> {
        match value {
            None | Some("1") | Some("on") | Some("yes") | Some("true") => Ok(true),
            Some("0") | Some("off") | Some("no") | Some("false") => Ok(false),
            _ => Err(ParamError::InvalidValue),
        }
    }
}

#[doc(hidden)]
pub fn setup_kv_param<S: ParamStorage>(storage: &S, occurrences: &[Option<&str>], name: &str)
where
    S::Value: ParseParamValue,
{
    let Some(last) = occurrences.last() else {
        return;
    };
    match last {
        Some(value) => match S::Value::parse_param(value) {
            Ok(v) => storage.store_param(v),
            Err(_) => ostd::warn!("invalid value for kernel parameter '{}'", name),
        },
        None => ostd::warn!("kernel parameter '{}' requires a value", name),
    }
}

#[doc(hidden)]
pub fn setup_repeatable_kv_param<S: ParamStorage>(
    storage: &S,
    occurrences: &[Option<&str>],
    name: &str,
) where
    S::Value: ParseRepeatableParamValue,
{
    let values: Vec<&str> = occurrences.iter().filter_map(|val| *val).collect();
    if values.is_empty() {
        ostd::warn!("repeatable parameter '{}' requires values", name);
        return;
    }
    match S::Value::parse_all(&values) {
        Ok(v) => storage.store_param(v),
        Err(_) => ostd::warn!("invalid value for kernel parameter '{}'", name),
    }
}

#[doc(hidden)]
pub fn setup_flag_param<S: ParamStorage>(storage: &S, occurrences: &[Option<&str>], name: &str)
where
    S::Value: ParseFlag,
{
    let last = match occurrences.last() {
        Some(val) => *val,
        None => return,
    };
    match S::Value::parse_flag(last) {
        Ok(v) => storage.store_param(v),
        Err(_) => ostd::warn!("invalid value for flag '{}'", name),
    }
}
