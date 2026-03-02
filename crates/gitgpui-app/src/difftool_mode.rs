use crate::cli::{DifftoolConfig, DifftoolInputKind, classify_difftool_input, exit_code};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::{Builder, TempDir};

/// Result of running the dedicated difftool mode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DifftoolRunResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// Execute difftool mode by delegating to `git diff --no-index`.
///
/// Git exits with code `1` when files differ, which is not an operational
/// failure for a diff tool. We normalize both `0` (no diff) and `1` (diff
/// present) to process success for the app-level contract.
pub fn run_difftool(config: &DifftoolConfig) -> Result<DifftoolRunResult, String> {
    let prepared_inputs = prepare_diff_inputs(config)?;

    let mut cmd = Command::new("git");
    cmd.arg("diff").arg("--no-index").arg("--no-ext-diff");
    // When launched from `git difftool`, Git sets `GIT_EXTERNAL_DIFF` to its
    // helper. Remove it so this nested `git diff --no-index` cannot recurse.
    cmd.env_remove("GIT_EXTERNAL_DIFF");
    let labels = resolve_labels(config);

    cmd.arg("--")
        .arg(&prepared_inputs.local)
        .arg(&prepared_inputs.remote);

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to launch `git diff --no-index`: {e}"))?;

    let status_code = output.status.code();
    let mut stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if let Some((left, right)) = labels {
        stdout = apply_labels_to_unified_diff_headers(&stdout, &left, &right);
    }

    match status_code {
        Some(0) => Ok(DifftoolRunResult {
            stdout,
            stderr,
            exit_code: exit_code::SUCCESS,
        }),
        Some(1) if !has_git_error_prefix(&stderr) => Ok(DifftoolRunResult {
            stdout,
            stderr,
            exit_code: exit_code::SUCCESS,
        }),
        Some(code) => {
            let detail = stderr.trim();
            if detail.is_empty() {
                Err(format!(
                    "`git diff --no-index` failed with exit code {code}"
                ))
            } else {
                Err(format!(
                    "`git diff --no-index` failed with exit code {code}: {detail}"
                ))
            }
        }
        None => Err("`git diff --no-index` terminated by signal".to_string()),
    }
}

struct PreparedDiffInputs {
    local: PathBuf,
    remote: PathBuf,
    _tempdir: Option<TempDir>,
}

fn prepare_diff_inputs(config: &DifftoolConfig) -> Result<PreparedDiffInputs, String> {
    let local_kind = classify_difftool_input(&config.local, "Local")?;
    let remote_kind = classify_difftool_input(&config.remote, "Remote")?;

    if local_kind != remote_kind {
        return Err(format!(
            "Difftool input kind mismatch: local is a {} and remote is a {}. Use two files or two directories.",
            display_input_kind(local_kind),
            display_input_kind(remote_kind)
        ));
    }

    if local_kind != DifftoolInputKind::Directory {
        return Ok(PreparedDiffInputs {
            local: config.local.clone(),
            remote: config.remote.clone(),
            _tempdir: None,
        });
    }

    let tempdir = Builder::new()
        .prefix("gitgpui-difftool-")
        .tempdir()
        .map_err(|e| format!("Failed to create temporary directory staging area: {e}"))?;

    let staged_local = tempdir.path().join("left");
    let staged_remote = tempdir.path().join("right");
    copy_tree_dereferencing_symlinks(&config.local, &staged_local)?;
    copy_tree_dereferencing_symlinks(&config.remote, &staged_remote)?;

    Ok(PreparedDiffInputs {
        local: staged_local,
        remote: staged_remote,
        _tempdir: Some(tempdir),
    })
}

fn display_input_kind(kind: DifftoolInputKind) -> &'static str {
    match kind {
        DifftoolInputKind::Directory => "directory",
        DifftoolInputKind::FileLike => "file",
    }
}

fn copy_tree_dereferencing_symlinks(src: &Path, dst: &Path) -> Result<(), String> {
    let mut active_dirs = HashSet::new();
    copy_tree_dereferencing_symlinks_inner(src, dst, &mut active_dirs)
}

fn copy_tree_dereferencing_symlinks_inner(
    src: &Path,
    dst: &Path,
    active_dirs: &mut HashSet<PathBuf>,
) -> Result<(), String> {
    let canonical_src = fs::canonicalize(src).map_err(|e| {
        format!(
            "Failed to resolve directory {} while staging directory diff inputs: {e}",
            src.display()
        )
    })?;
    if !active_dirs.insert(canonical_src.clone()) {
        return Err(format!(
            "Detected symlink cycle while staging directory diff inputs at {}",
            src.display()
        ));
    }

    let result = copy_tree_dereferencing_symlinks_impl(src, dst, active_dirs);
    active_dirs.remove(&canonical_src);
    result
}

fn copy_tree_dereferencing_symlinks_impl(
    src: &Path,
    dst: &Path,
    active_dirs: &mut HashSet<PathBuf>,
) -> Result<(), String> {
    fs::create_dir_all(dst)
        .map_err(|e| format!("Failed to create staged directory {}: {e}", dst.display()))?;

    let entries = fs::read_dir(src)
        .map_err(|e| format!("Failed to read directory {}: {e}", src.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("Failed to read entry in {}: {e}", src.display()))?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|e| format!("Failed to read file type for {}: {e}", src_path.display()))?;

        if file_type.is_dir() {
            copy_tree_dereferencing_symlinks_inner(&src_path, &dst_path, active_dirs)?;
            continue;
        }

        if file_type.is_symlink() {
            copy_symlink_target_contents(&src_path, &dst_path, active_dirs)?;
            continue;
        }

        if file_type.is_file() {
            fs::copy(&src_path, &dst_path).map_err(|e| {
                format!(
                    "Failed to stage file {} to {}: {e}",
                    src_path.display(),
                    dst_path.display()
                )
            })?;
            continue;
        }

        return Err(format!(
            "Unsupported entry type at {} while staging directory diff inputs",
            src_path.display()
        ));
    }

    Ok(())
}

fn copy_symlink_target_contents(
    link_path: &Path,
    dst_path: &Path,
    active_dirs: &mut HashSet<PathBuf>,
) -> Result<(), String> {
    let target = fs::read_link(link_path)
        .map_err(|e| format!("Failed to read symlink target {}: {e}", link_path.display()))?;
    let resolved_target = if target.is_absolute() {
        target.clone()
    } else {
        link_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(&target)
    };

    match fs::metadata(&resolved_target) {
        Ok(meta) if meta.is_file() => {
            fs::copy(&resolved_target, dst_path).map_err(|e| {
                format!(
                    "Failed to stage symlink target file {} to {}: {e}",
                    resolved_target.display(),
                    dst_path.display()
                )
            })?;
        }
        Ok(meta) if meta.is_dir() => {
            copy_tree_dereferencing_symlinks_inner(&resolved_target, dst_path, active_dirs)?;
        }
        _ => {
            write_symlink_target(dst_path, &target).map_err(|e| {
                format!(
                    "Failed to materialize unresolved symlink {} into {}: {e}",
                    link_path.display(),
                    dst_path.display()
                )
            })?;
        }
    }

    Ok(())
}

/// Write symlink target path bytes to a file, preserving non-UTF-8 content on Unix.
fn write_symlink_target(dst: &Path, target: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        fs::write(dst, target.as_os_str().as_bytes())
    }
    #[cfg(not(unix))]
    {
        fs::write(dst, target.to_string_lossy().as_bytes())
    }
}

fn has_git_error_prefix(stderr: &str) -> bool {
    stderr
        .lines()
        .map(str::trim_start)
        .any(|line| line.starts_with("error:") || line.starts_with("fatal:"))
}

fn push_labeled_header_line(out: &mut String, prefix: &str, label: &str, segment: &str) {
    out.push_str(prefix);
    out.push(' ');
    out.push_str(label);
    if segment.ends_with('\n') {
        out.push('\n');
    }
}

fn apply_labels_to_unified_diff_headers(diff: &str, left: &str, right: &str) -> String {
    let mut out = String::with_capacity(diff.len());
    // Rewrite only file header lines (`---` / `+++`) and never hunk payload.
    // Hunk lines can legally begin with `--- ` / `+++ ` when content itself
    // starts with `-- ` / `++ `, and those must remain untouched.
    let mut in_hunk = false;
    let mut awaiting_old_header = true;
    let mut awaiting_new_header = false;

    for segment in diff.split_inclusive('\n') {
        if segment.starts_with("diff --git ") {
            in_hunk = false;
            awaiting_old_header = true;
            awaiting_new_header = false;
            out.push_str(segment);
            continue;
        }

        if segment.starts_with("@@ ") {
            in_hunk = true;
            awaiting_old_header = false;
            awaiting_new_header = false;
            out.push_str(segment);
            continue;
        }

        if !in_hunk && awaiting_old_header && segment.starts_with("--- ") {
            push_labeled_header_line(&mut out, "---", left, segment);
            awaiting_old_header = false;
            awaiting_new_header = true;
            continue;
        }

        if !in_hunk && awaiting_new_header && segment.starts_with("+++ ") {
            push_labeled_header_line(&mut out, "+++", right, segment);
            awaiting_new_header = false;
            continue;
        }

        out.push_str(segment);
    }

    if !diff.ends_with('\n') && out.ends_with('\n') {
        out.pop();
    }

    out
}

fn resolve_labels(config: &DifftoolConfig) -> Option<(String, String)> {
    let has_custom_labels = config.label_left.is_some() || config.label_right.is_some();
    let has_display_path = config.display_path.is_some();
    if !has_custom_labels && !has_display_path {
        return None;
    }

    let left_default = config
        .display_path
        .as_ref()
        .map(|path| format!("a/{path}"))
        .unwrap_or_else(|| config.local.display().to_string());
    let right_default = config
        .display_path
        .as_ref()
        .map(|path| format!("b/{path}"))
        .unwrap_or_else(|| config.remote.display().to_string());

    let left = config.label_left.clone().unwrap_or(left_default);
    let right = config.label_right.clone().unwrap_or(right_default);
    Some((left, right))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn write_file(path: &std::path::Path, content: &str) {
        std::fs::write(path, content).expect("write fixture file");
    }

    fn write_bytes(path: &std::path::Path, content: &[u8]) {
        std::fs::write(path, content).expect("write fixture bytes");
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
        unix_fs::symlink(non_utf8_target, left.join("entry"))
            .expect("create non-UTF-8 symlink");

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
        let input =
            "diff --git a/l b/r\n--- a/l\n+++ b/r\n@@ -1 +1 @@\n--- content\n+++ content\n";
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

    #[test]
    fn has_git_error_prefix_detects_error_and_fatal_diagnostics() {
        assert!(has_git_error_prefix("error: unable to read file"));
        assert!(has_git_error_prefix(
            "warning: context\nfatal: cannot stat '/tmp/missing'"
        ));
    }

    #[test]
    fn has_git_error_prefix_ignores_non_error_output() {
        assert!(!has_git_error_prefix(
            "diff --git a/file b/file\nindex 1..2 100644"
        ));
        assert!(!has_git_error_prefix("warning: textconv failed"));
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
            result.stdout.contains("original line")
                || result.stdout.contains("changed line"),
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
        write_file(&right_dir.join("main.rs"), "fn main() { println!(\"hello\"); }\n");

        let result = run_difftool(&config(left_dir, right_dir)).expect("difftool run");
        assert_eq!(result.exit_code, exit_code::SUCCESS);
        assert!(
            result.stdout.contains("main.rs"),
            "directory diff output should mention the changed file: {}",
            result.stdout
        );
    }
}
