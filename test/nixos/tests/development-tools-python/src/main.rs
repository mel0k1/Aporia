use nixos_test_framework::*;

nixos_test_main!();

#[nixos_test]
fn python3_regrtest(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("py_src=/tmp/python3-src")?;
    nixos_shell.run_cmd("mkdir -p $py_src")?;
    nixos_shell.run_cmd("tar xf /tmp/python3-src.tar.xz --strip-components=1 -C $py_src")?;

    nixos_shell.run_cmd("pyhome=/tmp/pyhome")?;
    nixos_shell
        .run_cmd("cp -dR --preserve=mode $(python3 -c 'import sys; print(sys.prefix)') $pyhome")?;
    nixos_shell.run_cmd(
        "py_testdir=$pyhome/lib/python$(python3 -c 'import sys; print(\"%d.%d\" % sys.version_info[:2])')",
    )?;
    nixos_shell.run_cmd("rm -rf $py_testdir/test")?;
    nixos_shell.run_cmd("cp -a $py_src/Lib/test $py_testdir/")?;
    nixos_shell.run_cmd("cp -a $py_src/Tools $pyhome/lib/Tools")?;

    const RESULT_SUCCESS: &str = "Result: SUCCESS";

    let testcases =
        std::fs::read_to_string("src/testcases.txt").map_err(|err| Error::Pty(err.into()))?;
    let mut failed_tests = Vec::new();
    for testcase in testcases
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let cmd = format!("$pyhome/bin/python3 -m test -u all {testcase}");
        if let Err(err) = nixos_shell.run_cmd_and_expect(&cmd, RESULT_SUCCESS) {
            failed_tests.push((testcase.to_string(), err));
        }
    }

    if !failed_tests.is_empty() {
        println!("=== Failed Python 3 regression tests ===");
        for (test, _) in &failed_tests {
            println!("  - {test}");
        }
        println!("======================================");

        return Err(Error::Aggregated {
            summary: "Selected Python 3 regression tests failed!".to_string(),
            collections: failed_tests,
        });
    }

    Ok(())
}
