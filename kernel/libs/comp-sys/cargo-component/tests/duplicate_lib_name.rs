mod test_utils;

#[test]
fn duplicate_lib_name() {
    let stderr = run_cargo_component_cmd!();
    assert!(stderr.contains("duplicate library names"));
}
