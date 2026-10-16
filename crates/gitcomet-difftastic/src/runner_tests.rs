use super::*;

#[test]
#[cfg(unix)]
fn symlinked_application_uses_helpers_beside_the_real_executable() {
    let directory = tempfile::tempdir().unwrap();
    let bundle = directory.path().join("bundle");
    std::fs::create_dir_all(bundle.join("helpers")).unwrap();
    let executable = bundle.join("gitcomet");
    std::fs::write(&executable, b"app").unwrap();
    let helper = bundle.join("helpers/difft");
    std::fs::write(&helper, b"helper").unwrap();
    let alias = directory.path().join("gitcomet");
    std::os::unix::fs::symlink(&executable, &alias).unwrap();
    assert_eq!(
        bundled_executable_for(&alias).unwrap(),
        helper.canonicalize().unwrap()
    );
}

fn fixture(mode: &str, marker: &Path) -> Command {
    let mut command = command(&std::env::current_exe().unwrap());
    command
        .args([
            "--ignored",
            "--exact",
            "runner::tests::fixture_child",
            "--nocapture",
        ])
        .env("DIFFTASTIC_TEST_MODE", mode)
        .env("DIFFTASTIC_TEST_MARKER", marker);
    command
}

#[test]
#[ignore = "subprocess fixture"]
fn fixture_child() {
    let mode = std::env::var("DIFFTASTIC_TEST_MODE").unwrap();
    if mode == "large" {
        std::io::stdout()
            .write_all(&vec![b'x'; 128 * 1024])
            .unwrap();
    }
    std::fs::write(std::env::var_os("DIFFTASTIC_TEST_MARKER").unwrap(), b"ran").unwrap();
}

#[test]
fn cancellation_prevents_starting_the_process() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("marker");
    let token = CancellationToken::new();
    token.cancel();
    assert!(matches!(
        run(
            fixture("normal", &marker),
            &token,
            PROCESS_TIMEOUT,
            STDOUT_LIMIT
        ),
        Err(DifftasticError::Cancelled)
    ));
    assert!(!marker.exists());
    run(
        fixture("normal", &marker),
        &CancellationToken::new(),
        PROCESS_TIMEOUT,
        STDOUT_LIMIT,
    )
    .unwrap();
    assert_eq!(std::fs::read(&marker).unwrap(), b"ran");
}

#[test]
fn captures_json_sized_output_without_diagnostic_truncation() {
    let directory = tempfile::tempdir().unwrap();
    let output = run(
        fixture("large", &directory.path().join("marker")),
        &CancellationToken::new(),
        PROCESS_TIMEOUT,
        STDOUT_LIMIT,
    )
    .unwrap();
    assert!(output.stdout.len() >= 128 * 1024);
}

#[test]
fn oversized_output_is_rejected_instead_of_truncated() {
    let directory = tempfile::tempdir().unwrap();
    assert!(matches!(
        run(
            fixture("large", &directory.path().join("marker")),
            &CancellationToken::new(),
            PROCESS_TIMEOUT,
            1024
        ),
        Err(DifftasticError::OutputLimit)
    ));
}

#[test]
fn expired_deadline_terminates_and_reaps_the_child() {
    let directory = tempfile::tempdir().unwrap();
    assert!(matches!(
        run(
            fixture("normal", &directory.path().join("marker")),
            &CancellationToken::new(),
            Duration::ZERO,
            STDOUT_LIMIT
        ),
        Err(DifftasticError::Timeout)
    ));
}

#[test]
fn oversized_sources_fall_back_without_a_helper() {
    let text = "x".repeat(BYTE_LIMIT + 1);
    assert_eq!(
        compare(
            DiffInput {
                path: Path::new("file.rs"),
                old: "",
                new: &text
            },
            &CancellationToken::new()
        )
        .unwrap(),
        StructuralDiffOutcome::Fallback(DiffEngineFallback::ByteLimit)
    );
}

#[test]
#[ignore = "requires the pinned packaged helper; run after scripts/build-difftastic.py"]
fn packaged_helper_compares_javascript() {
    let StructuralDiffOutcome::Compared(diff) = compare(
        DiffInput {
            path: Path::new("example.js"),
            old: "let result = value;\n",
            new: "let result = wrap(value);\n",
        },
        &CancellationToken::new(),
    )
    .unwrap() else {
        panic!("expected a structural comparison from the packaged helper")
    };
    assert_eq!(diff.language, "JavaScript");
    assert_eq!(diff.aligned_lines, vec![(Some(0), Some(0))]);
    assert!(diff.old_changes.is_empty());
    assert_eq!(diff.new_changes[&0], vec![13..17, 17..18, 23..24]);
}
