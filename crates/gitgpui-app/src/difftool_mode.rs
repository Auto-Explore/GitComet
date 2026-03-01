use crate::cli::{DifftoolConfig, exit_code};
use std::process::Command;

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
    let mut cmd = Command::new("git");
    cmd.arg("diff").arg("--no-index");
    let labels = resolve_labels(config);

    cmd.arg("--").arg(&config.local).arg(&config.remote);

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

fn has_git_error_prefix(stderr: &str) -> bool {
    stderr
        .lines()
        .map(str::trim_start)
        .any(|line| line.starts_with("error:"))
}

fn apply_labels_to_unified_diff_headers(diff: &str, left: &str, right: &str) -> String {
    let mut out = String::with_capacity(diff.len());

    for segment in diff.split_inclusive('\n') {
        if segment.starts_with("--- ") {
            if segment.ends_with('\n') {
                out.push_str(&format!("--- {left}\n"));
            } else {
                out.push_str(&format!("--- {left}"));
            }
            continue;
        }
        if segment.starts_with("+++ ") {
            if segment.ends_with('\n') {
                out.push_str(&format!("+++ {right}\n"));
            } else {
                out.push_str(&format!("+++ {right}"));
            }
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

    fn config(local: PathBuf, remote: PathBuf) -> DifftoolConfig {
        DifftoolConfig {
            local,
            remote,
            display_path: None,
            label_left: None,
            label_right: None,
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
            err.contains("failed with exit code"),
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

    #[test]
    fn apply_labels_rewrites_unified_headers_only() {
        let input = "diff --git a/l b/r\n--- a/l\n+++ b/r\n@@ -1 +1 @@\n-a\n+b\n";
        let got = apply_labels_to_unified_diff_headers(input, "LEFT", "RIGHT");
        assert!(got.contains("diff --git a/l b/r"));
        assert!(got.contains("--- LEFT"));
        assert!(got.contains("+++ RIGHT"));
        assert!(got.contains("@@ -1 +1 @@"));
    }
}
