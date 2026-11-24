//! Support for the code coverage feature of OSDK.
//!
//! For more information about the code coverage feature (`cargo osdk run --coverage`),
//! check out the OSDK reference manual.

use alloc::vec::Vec;
use core::mem::ManuallyDrop;

use spin::Once;

use crate::sync::SpinLock;

/// A hook that is invoked when the system exits to dump the code coverage data.
pub(crate) fn on_system_exit() {
    static COVERAGE_DATA: Once<Vec<u8>> = Once::new();

    let coverage = COVERAGE_DATA.call_once(|| {
        let mut coverage = Vec::new();

        unsafe { minicov::capture_coverage(&mut coverage).unwrap() };
        coverage
    });

    crate::early_println!("#### Coverage: {:p} {}", coverage.as_ptr(), coverage.len());
}
