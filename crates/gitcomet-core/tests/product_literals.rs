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
    // Windows ships a `python3` app-execution alias that reports a missing
    // interpreter on stdout and exits non-zero rather than failing to spawn,
    // so a non-zero run is treated the same as "not installed" and the next
    // interpreter is tried. The last output kept is the real one: it either
    // passed or carries the checker's own diagnostics.
    let mut outputs = Vec::new();
    for interpreter in ["python3", "python"] {
        match run(interpreter) {
            Ok(output) => outputs.push(output),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                panic!("Python is required for the repository's product literal check: {error}")
            }
        }
        if outputs.last().is_some_and(|output| output.status.success()) {
            break;
        }
    }
    let output = outputs
        .pop()
        .expect("Python is required for the repository's product literal check");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
