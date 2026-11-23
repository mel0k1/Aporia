mod test_utils;

#[test]
fn test() {
    let stderr = run_cargo_component_cmd!();
    assert!(!stderr.contains("access controlled entry point is disallowed"));
}
