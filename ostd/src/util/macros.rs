#[macro_export]
macro_rules! const_assert {
    ($cond:expr $(,)?) => { const _: () = assert!($cond); };
    ($cond:expr, $($arg:tt)+) => { const _: () = assert!($cond, $($arg)*); };
}

#[macro_export]
macro_rules! ptr_null_of {
    ($expr:expr $(,)?) => {
        if true {
            core::ptr::null()
        } else {
            unreachable!();

            #[expect(unreachable_code)]
            unsafe {
                $expr
            }
        }
    };
}
