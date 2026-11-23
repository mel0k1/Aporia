use nixos_test_framework::*;

nixos_test_main!();

#[nixos_test]
fn irssi_connect(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect("expect /tmp/irssi.exp", "PASS")?;
    Ok(())
}

#[nixos_test]
fn weechat_connect(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect("expect /tmp/weechat.exp", "PASS")?;
    Ok(())
}
