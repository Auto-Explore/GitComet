//! Keep the Cargo acceptance check and CI on the same literal scanner.
use std::ffi::OsString;
use std::process::Command;

fn find_python(mut available: impl FnMut(&str) -> bool) -> Option<&'static str> {
    // Windows App Execution Aliases can spawn successfully but fail to run
    // Python. Check the interpreter before running the audit, never retry a
    // failed audit with a different interpreter.
    ["python3", "python"]
        .into_iter()
        .find(|name| available(name))
}

#[test]
fn product_literals_match_the_shrinking_allowlist() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let python = std::env::var_os("GITCOMET_TEST_PYTHON").unwrap_or_else(|| {
        OsString::from(
            find_python(|name| {
                Command::new(name)
                    .args(["-c", "import sys; assert sys.version_info >= (3, 8)"])
                    .output()
                    .is_ok_and(|output| output.status.success())
            })
            .expect("Python 3 is required; set GITCOMET_TEST_PYTHON to its executable"),
        )
    });
    let output = Command::new(python)
        .arg(root.join("scripts/ci/identity_literals.py"))
        .current_dir(root)
        .output()
        .expect("Python is required for the repository's product literal check");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn python_selection_skips_unavailable_interpreters_and_stops_at_the_first_working_one() {
    let mut tried = Vec::new();
    assert_eq!(
        find_python(|name| {
            tried.push(name.to_string());
            name == "python"
        }),
        Some("python")
    );
    assert_eq!(tried, ["python3", "python"]);
    assert_eq!(find_python(|name| name == "python3"), Some("python3"));
    assert_eq!(find_python(|_| false), None);
}
