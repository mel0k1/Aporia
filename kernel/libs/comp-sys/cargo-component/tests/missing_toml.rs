mod test_utils;

#[test]
fn missing_toml() {
    let stderr = run_cargo_component_cmd!();
    assert!(stderr.contains("cannot find components.toml"));
}
