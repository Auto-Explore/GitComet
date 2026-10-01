//! Keep the Cargo acceptance check and CI on the same literal scanner.
#[test]
fn product_literals_match_the_shrinking_allowlist() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let run = |python: &str| {
        std::process::Command::new(python)
            .arg(root.join("scripts/ci/identity_literals.py"))
            .current_dir(root)
            .output()
    };
    let output = run("python3")
        .or_else(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                run("python")
            } else {
                Err(error)
            }
        })
        .expect("Python is required for the repository's product literal check");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
