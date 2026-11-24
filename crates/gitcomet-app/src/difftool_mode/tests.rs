use super::*;
#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;

fn write_file(path: &std::path::Path, content: &str) {
    std::fs::write(path, content).expect("write fixture file");
}

fn write_bytes(path: &std::path::Path, content: &[u8]) {
    std::fs::write(path, content).expect("write fixture bytes");
}

#[cfg(unix)]
fn create_git_root_marker(repo_root: &Path) {
    let git_dir = repo_root.join(".git");
    std::fs::create_dir_all(&git_dir).expect("create git dir marker");
    write_file(&git_dir.join("HEAD"), "ref: refs/heads/main\n");
}

fn config(local: PathBuf, remote: PathBuf) -> DifftoolConfig {
    DifftoolConfig {
        local,
        remote,
        display_path: None,
        label_left: None,
        label_right: None,
        gui: false,
    }
}

#[test]
fn focused_source_preserves_full_text_crlf_unicode_and_missing_newline() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("input.rs");
    let text = "fn main() {\r\n    println!(\"héllo\");\r\n}";
    fs::write(&path, text).unwrap();
    assert_eq!(focused_source(&path).unwrap().as_deref(), Some(text));
}

#[test]
fn focused_source_keeps_binary_large_and_directory_inputs_on_patch_path() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(focused_source(dir.path()).unwrap(), None);
    let path = dir.path().join("input");
    fs::write(&path, [0, 1, 2, 0]).unwrap();
    assert_eq!(focused_source(&path).unwrap(), None);
    fs::write(
        &path,
        vec![b'a'; gitcomet_core::patch_action::SOURCE_BYTE_LIMIT + 1],
    )
    .unwrap();
    assert_eq!(focused_source(&path).unwrap(), None);
    fs::remove_file(&path).unwrap();
    assert!(focused_source(&path).is_err());
}

#[test]
#[cfg(feature = "ui-gpui-runtime")]
fn focused_input_retains_language_path_and_uses_patch_for_directories() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("before");
    let new = dir.path().join("after");
    fs::write(&old, "fn a() {}\n").unwrap();
    fs::write(&new, "fn b() {}\n").unwrap();
    let mut config = config(old, new);
    config.display_path = Some("src/code.rs".into());
    match focused_input(&config, "patch".into()) {
        gitcomet_ui_gpui::FocusedDiffInput::FilePair { path, old, new } => {
            assert_eq!(path, PathBuf::from("src/code.rs"));
            assert_eq!(&*old, "fn a() {}\n");
            assert_eq!(&*new, "fn b() {}\n");
        }
        gitcomet_ui_gpui::FocusedDiffInput::Patch(_) => {
            panic!("regular text needs full sources")
        }
    }
    config.local = dir.path().into();
    config.remote = dir.path().into();
    assert!(
        matches!(focused_input(&config, "patch".into()), gitcomet_ui_gpui::FocusedDiffInput::Patch(text) if text == "patch")
    );
}

#[test]
#[cfg(feature = "ui-gpui-runtime")]
fn focused_input_preserves_bom_changes_in_the_patch() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("old.rs");
    let new = dir.path().join("new.rs");
    fs::write(&old, "\u{feff}fn main() {}\n").unwrap();
    fs::write(&new, "fn main() {}\n").unwrap();
    let config = config(old, new);
    let patch = run_difftool(&config).unwrap().stdout;
    assert!(!patch.is_empty());
    assert!(
        matches!(focused_input(&config, patch.clone()), gitcomet_ui_gpui::FocusedDiffInput::Patch(text) if text == patch)
    );
}

#[test]
#[cfg(all(unix, feature = "ui-gpui-runtime"))]
fn focused_input_preserves_mode_only_changes_in_the_patch() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("old.rs");
    let new = dir.path().join("new.rs");
    fs::write(&old, "fn main() {}\n").unwrap();
    fs::write(&new, "fn main() {}\n").unwrap();
    fs::set_permissions(&old, fs::Permissions::from_mode(0o644)).unwrap();
    fs::set_permissions(&new, fs::Permissions::from_mode(0o755)).unwrap();
    let config = config(old, new);
    let patch = run_difftool(&config).unwrap().stdout;
    assert!(patch.contains("old mode 100644\nnew mode 100755"));
    assert!(
        matches!(focused_input(&config, patch.clone()), gitcomet_ui_gpui::FocusedDiffInput::Patch(text) if text == patch)
    );
}

#[test]
fn run_difftool_identical_files_returns_success_with_no_diff() {
    let tmp = tempfile::tempdir().unwrap();
    let left = tmp.path().join("left.txt");
    let right = tmp.path().join("right.txt");
    write_file(&left, "same\n");
    write_file(&right, "same\n");

    let result = run_difftool(&config(left, right)).expect("difftool run");
    assert_eq!(result.exit_code, exit_code::SUCCESS);
    assert!(
        result.stdout.trim().is_empty(),
        "identical files should produce no stdout diff, got: {}",
        result.stdout
    );
}

#[test]
fn run_difftool_changed_files_maps_git_exit_1_to_success() {
    let tmp = tempfile::tempdir().unwrap();
    let left = tmp.path().join("left.txt");
    let right = tmp.path().join("right.txt");
    write_file(&left, "left\n");
    write_file(&right, "right\n");

    let result = run_difftool(&config(left, right)).expect("difftool run");
    assert_eq!(result.exit_code, exit_code::SUCCESS);
    assert!(
        result.stdout.contains("@@"),
        "expected a hunk in diff output"
    );
    assert!(result.stdout.contains("-left"));
    assert!(result.stdout.contains("+right"));
}

#[test]
fn run_difftool_uses_display_path_labels() {
    let tmp = tempfile::tempdir().unwrap();
    let left = tmp.path().join("left.txt");
    let right = tmp.path().join("right.txt");
    write_file(&left, "left\n");
    write_file(&right, "right\n");

    let mut cfg = config(left, right);
    cfg.display_path = Some("src/lib.rs".to_string());

    let result = run_difftool(&cfg).expect("difftool run");
    assert!(result.stdout.contains("--- a/src/lib.rs"));
    assert!(result.stdout.contains("+++ b/src/lib.rs"));
}

#[test]
fn run_difftool_uses_explicit_labels() {
    let tmp = tempfile::tempdir().unwrap();
    let left = tmp.path().join("left.txt");
    let right = tmp.path().join("right.txt");
    write_file(&left, "left\n");
    write_file(&right, "right\n");

    let mut cfg = config(left, right);
    cfg.label_left = Some("OURS".to_string());
    cfg.label_right = Some("THEIRS".to_string());

    let result = run_difftool(&cfg).expect("difftool run");
    assert!(result.stdout.contains("--- OURS"));
    assert!(result.stdout.contains("+++ THEIRS"));
}

#[test]
fn run_difftool_nonexistent_input_returns_error() {
    let tmp = tempfile::tempdir().unwrap();
    let left = tmp.path().join("missing.txt");
    let right = tmp.path().join("right.txt");
    write_file(&right, "right\n");

    let err = run_difftool(&config(left, right)).expect_err("expected error");
    assert!(
        err.contains("Local path does not exist")
            || err.contains("Failed to read metadata for local path")
            || err.contains("failed with exit code"),
        "unexpected error message: {err}"
    );
}

/// A refused input exits 1 like a difference does, but prints no diff;
/// its translated message must not read as one.
#[cfg(unix)]
#[test]
fn run_difftool_reports_a_git_error_in_any_locale() {
    gitcomet_core::test_support::localized_git::run(
        module_path!(),
        "run_difftool_reports_a_git_error_in_any_locale",
        r#"#!/bin/sh
for argument in "$@"; do
    if [ "$argument" = --no-index ]; then
        printf '%s\n' 'Fehler: konnte right.txt nicht lesen: Keine Berechtigung' >&2
        exit 1
    fi
done
exec git "$@"
"#,
        || {
            let tmp = tempfile::tempdir().unwrap();
            let left = tmp.path().join("left.txt");
            let right = tmp.path().join("right.txt");
            write_file(&left, "left\n");
            write_file(&right, "right\n");

            run_difftool(&config(left, right)).expect_err("a Git error is not a diff");
        },
    );
}

/// A textconv driver can render two different files alike: Git then exits
/// 1 with no diff and no message.
#[cfg(unix)]
#[test]
fn run_difftool_treats_a_silent_exit_1_as_a_difference() {
    gitcomet_core::test_support::localized_git::run(
        module_path!(),
        "run_difftool_treats_a_silent_exit_1_as_a_difference",
        r#"#!/bin/sh
for argument in "$@"; do
    if [ "$argument" = --no-index ]; then
        exit 1
    fi
done
exec git "$@"
"#,
        || {
            let tmp = tempfile::tempdir().unwrap();
            let left = tmp.path().join("left.docx");
            let right = tmp.path().join("right.docx");
            write_file(&left, "left\n");
            write_file(&right, "right\n");

            let result = run_difftool(&config(left, right)).expect("not an error");
            assert_eq!(result.exit_code, exit_code::SUCCESS);
        },
    );
}

#[test]
fn run_difftool_directory_diff_returns_success() {
    let tmp = tempfile::tempdir().unwrap();
    let left = tmp.path().join("left");
    let right = tmp.path().join("right");
    std::fs::create_dir_all(&left).unwrap();
    std::fs::create_dir_all(&right).unwrap();
    write_file(&left.join("a.txt"), "left\n");
    write_file(&right.join("a.txt"), "right\n");

    let result = run_difftool(&config(left, right)).expect("difftool run");
    assert_eq!(result.exit_code, exit_code::SUCCESS);
    assert!(
        result.stdout.contains("a.txt"),
        "expected filename in dir diff output, got: {}",
        result.stdout
    );
}

#[test]
fn prepare_diff_inputs_directory_without_symlinks_skips_staging_copy() {
    let tmp = tempfile::tempdir().unwrap();
    let left = tmp.path().join("left");
    let right = tmp.path().join("right");
    std::fs::create_dir_all(&left).unwrap();
    std::fs::create_dir_all(&right).unwrap();
    write_file(&left.join("a.txt"), "left\n");
    write_file(&right.join("a.txt"), "right\n");

    let prepared =
        prepare_diff_inputs(&config(left.clone(), right.clone())).expect("prepare inputs");
    assert_eq!(prepared.local, left);
    assert_eq!(prepared.remote, right);
    assert!(prepared._tempdir.is_none());
}

#[cfg(unix)]
#[test]
fn prepare_diff_inputs_directory_with_symlink_uses_staging_copy() {
    use std::os::unix::fs as unix_fs;

    let tmp = tempfile::tempdir().unwrap();
    let left = tmp.path().join("left");
    let right = tmp.path().join("right");
    std::fs::create_dir_all(&left).unwrap();
    std::fs::create_dir_all(&right).unwrap();
    write_file(&left.join("a.txt"), "before\n");
    write_file(&right.join("target.txt"), "after\n");
    unix_fs::symlink(right.join("target.txt"), right.join("a.txt"))
        .expect("create symlink in right dir");

    let prepared =
        prepare_diff_inputs(&config(left.clone(), right.clone())).expect("prepare inputs");
    assert_ne!(prepared.local, left);
    assert_ne!(prepared.remote, right);
    assert!(prepared._tempdir.is_some());

    let tempdir = prepared._tempdir.as_ref().unwrap();
    assert!(prepared.local.starts_with(tempdir.path()));
    assert!(prepared.remote.starts_with(tempdir.path()));
}

#[cfg(unix)]
#[test]
fn run_difftool_directory_diff_dereferences_symlinked_files() {
    use std::os::unix::fs as unix_fs;

    let tmp = tempfile::tempdir().unwrap();
    let left = tmp.path().join("left");
    let right = tmp.path().join("right");
    std::fs::create_dir_all(&left).unwrap();
    std::fs::create_dir_all(&right).unwrap();

    write_file(&left.join("a.txt"), "before\n");
    write_file(&right.join("target.txt"), "after\n");
    unix_fs::symlink(right.join("target.txt"), right.join("a.txt"))
        .expect("create symlink in right dir");

    let result = run_difftool(&config(left, right)).expect("difftool run");
    assert_eq!(result.exit_code, exit_code::SUCCESS);
    assert!(
        result.stdout.contains("-before") && result.stdout.contains("+after"),
        "expected dereferenced content diff, got: {}",
        result.stdout
    );
    assert!(
        !result.stdout.contains("new file mode 120000"),
        "did not expect symlink mode-only diff, got: {}",
        result.stdout
    );
}

#[cfg(unix)]
#[test]
fn run_difftool_directory_symlink_inputs_use_directory_content_diff() {
    use std::os::unix::fs as unix_fs;

    let tmp = tempfile::tempdir().unwrap();
    let left_dir = tmp.path().join("left");
    let right_dir = tmp.path().join("right");
    let left_link = tmp.path().join("left-link");
    let right_link = tmp.path().join("right-link");

    std::fs::create_dir_all(&left_dir).unwrap();
    std::fs::create_dir_all(&right_dir).unwrap();
    write_file(&left_dir.join("a.txt"), "before\n");
    write_file(&right_dir.join("a.txt"), "after\n");
    unix_fs::symlink(&left_dir, &left_link).expect("create symlink to left directory");
    unix_fs::symlink(&right_dir, &right_link).expect("create symlink to right directory");

    let result = run_difftool(&config(left_link, right_link)).expect("difftool run");
    assert_eq!(result.exit_code, exit_code::SUCCESS);
    assert!(
        result.stdout.contains("-before") && result.stdout.contains("+after"),
        "expected staged directory content diff, got: {}",
        result.stdout
    );
    assert!(
        !result.stdout.contains("new file mode 120000"),
        "did not expect top-level symlink-mode-only diff, got: {}",
        result.stdout
    );
}

#[cfg(unix)]
#[test]
fn run_difftool_directory_diff_rejects_symlink_cycles() {
    use std::os::unix::fs as unix_fs;

    let tmp = tempfile::tempdir().unwrap();
    let left = tmp.path().join("left");
    let right = tmp.path().join("right");
    std::fs::create_dir_all(&left).unwrap();
    std::fs::create_dir_all(&right).unwrap();

    write_file(&left.join("a.txt"), "left\n");
    write_file(&right.join("a.txt"), "right\n");
    unix_fs::symlink(".", right.join("loop")).expect("create self-referential symlink");

    let err = run_difftool(&config(left, right)).expect_err("expected symlink cycle error");
    assert!(
        err.contains("symlink cycle"),
        "expected cycle-specific error, got: {err}"
    );
}

#[cfg(unix)]
#[test]
fn run_difftool_directory_diff_rejects_symlink_target_outside_allowed_roots() {
    use std::os::unix::fs as unix_fs;

    let tmp = tempfile::tempdir().unwrap();
    let outside_tmp = tempfile::Builder::new()
        .prefix("gitcomet-difftool-outside-")
        .tempdir()
        .expect("create outside temp dir");
    let left = tmp.path().join("left");
    let right = tmp.path().join("right");
    let outside = outside_tmp.path().join("outside");
    std::fs::create_dir_all(&left).unwrap();
    std::fs::create_dir_all(&right).unwrap();
    std::fs::create_dir_all(&outside).unwrap();

    write_file(&left.join("a.txt"), "left\n");
    write_file(&outside.join("secret.txt"), "outside\n");
    unix_fs::symlink(outside.join("secret.txt"), right.join("a.txt"))
        .expect("create out-of-bound symlink");

    let err = run_difftool(&config(left, right)).expect_err("expected allowed-root error");
    assert!(
        err.contains("outside allowed roots"),
        "expected allowed-root-specific error, got: {err}"
    );
}

#[cfg(unix)]
#[test]
fn run_difftool_directory_diff_allows_symlink_target_within_detected_git_root() {
    use std::os::unix::fs as unix_fs;

    let tmp = tempfile::tempdir().unwrap();
    let repo_root = tmp.path().join("repo");
    let left = repo_root.join("left");
    let right = repo_root.join("right");
    let shared = repo_root.join("shared");
    create_git_root_marker(&repo_root);
    std::fs::create_dir_all(&left).unwrap();
    std::fs::create_dir_all(&right).unwrap();
    std::fs::create_dir_all(&shared).unwrap();

    write_file(&left.join("a.txt"), "before\n");
    write_file(&shared.join("target.txt"), "after\n");
    unix_fs::symlink(shared.join("target.txt"), right.join("a.txt"))
        .expect("create in-repo symlink");

    let result = run_difftool(&config(left, right)).expect("difftool run");
    assert_eq!(result.exit_code, exit_code::SUCCESS);
    assert!(
        result.stdout.contains("-before") && result.stdout.contains("+after"),
        "expected dereferenced in-repo symlink content diff, got: {}",
        result.stdout
    );
}

#[test]
fn run_difftool_binary_content_returns_success() {
    let tmp = tempfile::tempdir().unwrap();
    let left = tmp.path().join("left.bin");
    let right = tmp.path().join("right.bin");
    write_bytes(&left, &[0x00, 0x01, 0x02, 0x03]);
    write_bytes(&right, &[0x00, 0x01, 0xFF, 0x03]);

    let result = run_difftool(&config(left, right)).expect("difftool run");
    assert_eq!(result.exit_code, exit_code::SUCCESS);
    assert!(
        result.stdout.contains("Binary files")
            || result.stdout.contains("GIT binary patch")
            || result.stdout.contains("differ"),
        "expected binary diff output, got: {}",
        result.stdout
    );
}

#[test]
fn run_difftool_non_utf8_text_content_returns_success() {
    let tmp = tempfile::tempdir().unwrap();
    let left = tmp.path().join("left.dat");
    let right = tmp.path().join("right.dat");
    write_bytes(&left, b"prefix\n\xFF\n");
    write_bytes(&right, b"prefix\n\xFE\n");

    let result = run_difftool(&config(left, right)).expect("difftool run");
    assert_eq!(result.exit_code, exit_code::SUCCESS);
    assert!(
        !result.stdout.trim().is_empty(),
        "expected non-empty diff output for non-UTF8 content"
    );
}

#[cfg(unix)]
#[test]
fn run_difftool_directory_diff_preserves_non_utf8_broken_symlink_target_bytes() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs as unix_fs;

    let tmp = tempfile::tempdir().unwrap();
    let left = tmp.path().join("left");
    let right = tmp.path().join("right");
    std::fs::create_dir_all(&left).unwrap();
    std::fs::create_dir_all(&right).unwrap();

    // Non-UTF-8 bytes that to_string_lossy() would corrupt.
    let non_utf8_bytes = b"target-\xff-\xfe";
    let non_utf8_target = OsStr::from_bytes(non_utf8_bytes);

    // Left side: broken symlink with non-UTF-8 target (materialized as target bytes).
    unix_fs::symlink(non_utf8_target, left.join("entry")).expect("create non-UTF-8 symlink");

    // Right side: file containing the exact same raw bytes.
    write_bytes(&right.join("entry"), non_utf8_bytes);

    let result = run_difftool(&config(left.clone(), right)).expect("difftool run");
    assert_eq!(result.exit_code, exit_code::SUCCESS);
    assert!(
        result.stdout.trim().is_empty(),
        "expected no diff when broken symlink target bytes match file content, got: {}",
        result.stdout
    );
}

#[test]
fn apply_labels_rewrites_unified_headers_only() {
    let input = "diff --git a/l b/r\n--- a/l\n+++ b/r\n@@ -1 +1 @@\n-a\n+b\n";
    let got = apply_labels_to_unified_diff_headers(input, "LEFT", "RIGHT");
    assert!(got.contains("diff --git a/l b/r"));
    assert!(got.contains("--- LEFT"));
    assert!(got.contains("+++ RIGHT"));
    assert!(got.contains("@@ -1 +1 @@"));
}

#[test]
fn apply_labels_does_not_rewrite_hunk_content_that_looks_like_headers() {
    let input = "diff --git a/l b/r\n--- a/l\n+++ b/r\n@@ -1 +1 @@\n--- content\n+++ content\n";
    let got = apply_labels_to_unified_diff_headers(input, "LEFT", "RIGHT");
    assert!(got.contains("--- LEFT\n+++ RIGHT\n"));
    assert!(got.contains("@@ -1 +1 @@\n--- content\n+++ content\n"));
}

#[test]
fn apply_labels_rewrites_each_file_header_pair_in_multi_file_diff() {
    let input = concat!(
        "diff --git a/l b/l\n",
        "--- a/l\n",
        "+++ b/l\n",
        "@@ -1 +1 @@\n",
        "-a\n",
        "+b\n",
        "diff --git a/r b/r\n",
        "--- a/r\n",
        "+++ b/r\n",
        "@@ -1 +1 @@\n",
        "-c\n",
        "+d\n",
    );
    let got = apply_labels_to_unified_diff_headers(input, "LEFT", "RIGHT");
    assert_eq!(got.matches("--- LEFT").count(), 2);
    assert_eq!(got.matches("+++ RIGHT").count(), 2);
}

// ── Subdirectory invocation ─────────────────────────────────────

#[test]
fn run_difftool_files_in_nested_subdirectory() {
    // Simulates `git difftool` invoked from a subdirectory: diff inputs
    // are in a nested path, not the root temp directory.
    let tmp = tempfile::tempdir().unwrap();
    let subdir = tmp.path().join("src").join("components");
    std::fs::create_dir_all(&subdir).unwrap();

    let left = subdir.join("widget_LOCAL.txt");
    let right = subdir.join("widget_REMOTE.txt");
    write_file(&left, "original line\n");
    write_file(&right, "changed line\n");

    let result = run_difftool(&config(left, right)).expect("difftool run");
    assert_eq!(result.exit_code, exit_code::SUCCESS);
    assert!(
        result.stdout.contains("original line") || result.stdout.contains("changed line"),
        "diff output should contain file content: {}",
        result.stdout
    );
}

#[test]
fn run_difftool_files_in_different_directories() {
    // Input files are in different directories, simulating writeToTemp
    // mode where Git places stage files in a temp directory.
    let tmp = tempfile::tempdir().unwrap();
    let dir_a = tmp.path().join("stages");
    let dir_b = tmp.path().join("workdir").join("src");
    std::fs::create_dir_all(&dir_a).unwrap();
    std::fs::create_dir_all(&dir_b).unwrap();

    let left = dir_a.join("file_LOCAL_12345.txt");
    let right = dir_b.join("file.txt");
    write_file(&left, "before\n");
    write_file(&right, "after\n");

    let result = run_difftool(&config(left, right)).expect("difftool run");
    assert_eq!(result.exit_code, exit_code::SUCCESS);
    assert!(
        !result.stdout.is_empty(),
        "diff should produce output for different files"
    );
}

#[test]
fn run_difftool_directory_diff_in_nested_subdirectories() {
    // Directory diff mode with directories nested inside subdirectories.
    let tmp = tempfile::tempdir().unwrap();
    let left_dir = tmp.path().join("workspace").join("left");
    let right_dir = tmp.path().join("workspace").join("right");
    std::fs::create_dir_all(&left_dir).unwrap();
    std::fs::create_dir_all(&right_dir).unwrap();

    write_file(&left_dir.join("main.rs"), "fn main() {}\n");
    write_file(
        &right_dir.join("main.rs"),
        "fn main() { println!(\"hello\"); }\n",
    );

    let result = run_difftool(&config(left_dir, right_dir)).expect("difftool run");
    assert_eq!(result.exit_code, exit_code::SUCCESS);
    assert!(
        result.stdout.contains("main.rs"),
        "directory diff output should mention the changed file: {}",
        result.stdout
    );
}

#[test]
fn latin1_diff_output_decodes_instead_of_escaping() {
    let raw = b"diff --git a/a b/b\n@@ -1 +1 @@\n-caf\xe9\n+caf\xe9s\n".to_vec();
    assert_eq!(
        decode_unified_diff_output(raw),
        "diff --git a/a b/b\n@@ -1 +1 @@\n-café\n+cafés\n"
    );
    assert_eq!(decode_unified_diff_output(b"+ok\n".to_vec()), "+ok\n");
}

#[test]
fn latin1_diff_output_keeps_line_endings_as_utf8_output_does() {
    let crlf = b"diff --git a/a b/b\n@@ -1,2 +1,2 @@\n-caf\xe9\r\n+caf\xe9s\r\n same\r\n";
    assert_eq!(
        decode_unified_diff_output(crlf.to_vec()),
        "diff --git a/a b/b\n@@ -1,2 +1,2 @@\n-café\r\n+cafés\r\n same\r\n"
    );
    let unterminated = b"diff --git a/a b/b\n@@ -1 +1 @@\n-caf\xe9\n+caf\xe9s";
    assert_eq!(
        decode_unified_diff_output(unterminated.to_vec()),
        "diff --git a/a b/b\n@@ -1 +1 @@\n-café\n+cafés"
    );
}

#[test]
fn bytes_to_text_preserving_utf8_handles_pure_ascii() {
    assert_eq!(bytes_to_text_preserving_utf8(b"hello world"), "hello world");
}

#[test]
fn bytes_to_text_preserving_utf8_handles_valid_utf8() {
    let input = "café ñ 日本語".as_bytes();
    assert_eq!(bytes_to_text_preserving_utf8(input), "café ñ 日本語");
}

#[test]
fn bytes_to_text_preserving_utf8_escapes_invalid_bytes() {
    let input = b"good\xff\xfebad";
    let result = bytes_to_text_preserving_utf8(input);
    assert_eq!(result, "good\\xff\\xfebad");
}

#[test]
fn bytes_to_text_preserving_utf8_handles_empty() {
    assert_eq!(bytes_to_text_preserving_utf8(b""), "");
}

#[test]
fn bytes_to_text_preserving_utf8_escapes_leading_invalid() {
    let result = bytes_to_text_preserving_utf8(b"\x80rest");
    assert_eq!(result, "\\x80rest");
}
