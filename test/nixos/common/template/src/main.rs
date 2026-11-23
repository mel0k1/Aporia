use nixos_test_framework::*;

nixos_test_main!();

#[nixos_test]
fn echo_write_file(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("echo 'Hello, World!' > out.txt")?;
    nixos_shell.run_cmd_and_expect("ls out.txt", "out.txt")?;

    Ok(())
}
