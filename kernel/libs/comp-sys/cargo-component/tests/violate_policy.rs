


//! This test checks that if controlled resource not in whitelist is visited, cargo-component will
//! report warning message.

mod test_utils;

#[test]
fn violate_policy() {
    let stderr = run_cargo_component_cmd!();
    assert!(stderr.contains("access controlled entry point is disallowed"));
    assert!(stderr.contains("access foo3::foo_add in bar3"));
    assert!(stderr.contains("access foo3::FOO_ITEM in bar3"));
}
