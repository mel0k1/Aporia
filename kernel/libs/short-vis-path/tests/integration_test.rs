use std::process::Command;

struct TestCase {
    project_dir: String,
    cargo_expand_stdout: String,
}

impl TestCase {
    fn new(project_dir: &str) -> Self {
        let output = Command::new("cargo")
            .arg("expand")
            .current_dir(project_dir)
            .output()
            .unwrap();

        TestCase {
            project_dir: project_dir.to_owned(),
            cargo_expand_stdout: String::from_utf8(output.stdout).unwrap(),
        }
    }

    fn assert_contains(&self, text: &str) {
        let dir = &self.project_dir;
        let stdout = &self.cargo_expand_stdout;
        assert!(
            stdout.contains(text),
            "[project: {dir}] expected code `{text}` should be in the macro expansion:\n`{stdout}`"
        );
    }
}

#[test]
fn syntax_and_behavior() {
    let case = TestCase::new("./tests/syntax-and-behavior");
    let expected = [
        "pub(in crate::test_deepest_module_wins::parent::child::parent) const fn deepest_wins",
        "pub(in crate::test_override::parent) const fn override_parent",
        "pub(in crate::test_mod_rs_flavor) type RecognizeModRs",
        "pub(in crate::test_multiple_arguments) type VisibleToParent",
        "pub(in crate::test_multiple_arguments) type VisibleToParentToo",
        "pub(in crate::test_multiple_arguments) type VisibleToParent",
        "pub(in crate::test_multiple_arguments::two_idents) type VisibleToCurrent",
        "pub(in crate::test_outer_attribute) type VisibleToParent",
    ];

    for text in expected {
        case.assert_contains(text);
    }
}
