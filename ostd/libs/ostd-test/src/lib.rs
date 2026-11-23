#![cfg_attr(not(test), no_std)]

extern crate alloc;
#[cfg(not(test))]
use alloc::{boxed::Box, string::String};

#[derive(Clone, Debug)]
pub struct PanicInfo {
    pub message: String,
    pub file: String,
    pub line: usize,
    pub col: usize,
}

impl core::fmt::Display for PanicInfo {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        writeln!(f, "Panicked at {}:{}:{}", self.file, self.line, self.col)?;
        writeln!(f, "{}", self.message)
    }
}

#[derive(Clone)]
pub enum KtestError {
    Panic(Box<PanicInfo>),
    ShouldPanicButNoPanic,
    ExpectedPanicNotMatch(&'static str, Box<PanicInfo>),
    Unknown,
}

#[repr(C)]
#[derive(Clone, Debug, PartialEq)]
pub struct KtestItemInfo {
    pub module_path: &'static str,
    pub fn_name: &'static str,
    pub package: &'static str,
    pub source: &'static str,
    pub line: usize,
    pub col: usize,
}

#[repr(C)]
#[derive(Clone, Debug)]
pub struct KtestItem {
    fn_: fn() -> (),
    should_panic: (bool, Option<&'static str>),
    info: KtestItemInfo,
}

type CatchUnwindImpl = fn(f: fn() -> ()) -> Result<(), Box<dyn core::any::Any + Send>>;

impl KtestItem {
    #[doc(hidden)]
    pub const fn new(
        fn_: fn() -> (),
        should_panic: (bool, Option<&'static str>),
        info: KtestItemInfo,
    ) -> Self {
        Self {
            fn_,
            should_panic,
            info,
        }
    }

    pub fn info(&self) -> &KtestItemInfo {
        &self.info
    }

    pub fn run(&self, catch_unwind_impl: &CatchUnwindImpl) -> Result<(), KtestError> {
        let test_result = catch_unwind_impl(self.fn_);
        if !self.should_panic.0 {
            match test_result {
                Ok(()) => Ok(()),
                Err(e) => match e.downcast::<PanicInfo>() {
                    Ok(s) => Err(KtestError::Panic(s)),
                    Err(_payload) => Err(KtestError::Unknown),
                },
            }
        } else {
            match test_result {
                Ok(()) => Err(KtestError::ShouldPanicButNoPanic),
                Err(e) => match e.downcast::<PanicInfo>() {
                    Ok(s) => {
                        if let Some(expected) = self.should_panic.1 {
                            if s.message.contains(expected) {
                                Ok(())
                            } else {
                                Err(KtestError::ExpectedPanicNotMatch(expected, s))
                            }
                        } else {
                            Ok(())
                        }
                    }
                    Err(_payload) => Err(KtestError::Unknown),
                },
            }
        }
    }
}

macro_rules! ktest_array {
    () => {{
        unsafe extern "C" {
            fn __ktest_array();
            fn __ktest_array_end();
        }
        let array_ptr = __ktest_array as *const () as *const KtestItem;
        let array_end_ptr = __ktest_array_end as *const () as *const KtestItem;
        let l = unsafe { array_end_ptr.offset_from(array_ptr) as usize };
        unsafe { core::slice::from_raw_parts(array_ptr, l) }
    }};
}

pub struct KtestIter {
    index: usize,
}

impl Default for KtestIter {
    fn default() -> Self {
        Self::new()
    }
}

impl KtestIter {
    pub fn new() -> Self {
        Self { index: 0 }
    }
}

impl Iterator for KtestIter {
    type Item = KtestItem;

    fn next(&mut self) -> Option<Self::Item> {
        let ktest_item = ktest_array!().get(self.index)?;
        self.index += 1;
        Some(ktest_item.clone())
    }
}

unsafe extern "Rust" {
    static KTEST_TEST_WHITELIST: Option<&'static [&'static str]>;
    static KTEST_CRATE_WHITELIST: Option<&'static [&'static str]>;
}

pub fn get_ktest_test_whitelist() -> Option<&'static [&'static str]> {
    unsafe { KTEST_TEST_WHITELIST }
}

pub fn get_ktest_crate_whitelist() -> Option<&'static [&'static str]> {
    unsafe { KTEST_CRATE_WHITELIST }
}
