//! The test suite for Go standard library on Asterinas NixOS.
//!
//! Supplements the `Go` coverage in `development-tools`.
//!
//! See `test/nixos/README.md#documentation-maintenance` for sync requirements
//! between this test suite and the corresponding "Verified Usage" book section.

use nixos_test_framework::*;

nixos_test_main!();

#[nixos_test]
fn go_stdlib(nixos_shell: &mut Session) -> Result<(), Error> {
    let success_regex = Regex::new(r"(?m)^(?:ok|\?)\s+").map_err(|err| Error::Pty(err.into()))?;

    let testcases =
        std::fs::read_to_string("src/testcases.txt").map_err(|err| Error::Pty(err.into()))?;
    let mut failed_tests = Vec::new();
    for testcase in testcases
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let cmd = format!("go test -short {testcase}");
        if let Err(err) = nixos_shell.run_cmd_and_expect_regex(&cmd, &success_regex) {
            failed_tests.push((testcase.to_string(), err));
        }
    }

    if !failed_tests.is_empty() {
        println!("=== Failed Go stdlib tests ===");
        for (test, _) in &failed_tests {
            println!("  - {test}");
        }
        println!("==============================");

        return Err(Error::Aggregated {
            summary: "Selected Go stdlib tests failed!".to_string(),
            collections: failed_tests,
        });
    }

    Ok(())
}
