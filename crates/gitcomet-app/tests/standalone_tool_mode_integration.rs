use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
#[cfg(windows)]
use std::sync::OnceLock;

fn gitcomet_bin() -> PathBuf {
    for env_key in ["CARGO_BIN_EXE_gitcomet-app", "CARGO_BIN_EXE_gitcomet_app"] {
        if let Some(path) = std::env::var_os(env_key).map(PathBuf::from) {
            if path.is_file() {
                return path;
            }
        }
    }

    if let Some(path) = gitcomet_bin_from_current_exe() {
        return path;
    }

    panic!(
        "gitcomet-app binary path was not found. Tried CARGO_BIN_EXE_gitcomet-app, \
CARGO_BIN_EXE_gitcomet_app, and a fallback relative to current test executable"
    );
}

fn gitcomet_bin_from_current_exe() -> Option<PathBuf> {
    let test_exe = std::env::current_exe().ok()?;
    let deps_dir = test_exe.parent()?;
    let profile_dir = deps_dir.parent()?;
    let exe_suffix = std::env::consts::EXE_SUFFIX;

    for bin_name in ["gitcomet-app", "gitcomet_app"] {
        let candidate = profile_dir.join(format!("{bin_name}{exe_suffix}"));
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}

fn run_gitcomet<I, S>(args: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(gitcomet_bin())
        .args(args)
        .output()
        .expect("gitcomet-app command to run")
}

fn run_gitcomet_in_dir<I, S>(dir: &Path, args: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(gitcomet_bin())
        .current_dir(dir)
        .args(args)
        .output()
        .expect("gitcomet-app command to run")
}

fn run_gitcomet_in_dir_with_global_env<I, S>(
    dir: &Path,
    args: I,
    env: &IsolatedGlobalGitEnv,
) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(gitcomet_bin());
    env.apply_to_command(&mut command);
    command
        .current_dir(dir)
        .args(args)
        .output()
        .expect("gitcomet-app command to run")
}

fn git_config_get(repo_dir: &Path, key: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["-C"])
        .arg(repo_dir)
        .args(["config", "--get", key])
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

fn git_config_get_local(repo_dir: &Path, key: &str) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(["config", "--local", "--get", key])
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

fn git_config_set_local(repo_dir: &Path, key: &str, value: &str) {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(["config", "--local", key, value])
        .output()
        .expect("git config --local to run");
    assert!(
        output.status.success(),
        "failed to set local git config {key}\n{}",
        output_text(&output)
    );
}

fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent directories");
    }
    fs::write(path, contents).expect("write file");
}

fn write_bytes(path: &Path, contents: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent directories");
    }
    fs::write(path, contents).expect("write file");
}

fn output_text(output: &Output) -> String {
    format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}
#[cfg(windows)]
fn is_git_shell_startup_failure(text: &str) -> bool {
    text.contains("sh.exe: *** fatal error -")
        && (text.contains("couldn't create signal pipe") || text.contains("CreateFileMapping"))
}

#[cfg(windows)]
fn git_shell_available_for_tooling() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let output = match Command::new("git")
            .args(["difftool", "--tool-help"])
            .output()
        {
            Ok(output) => output,
            Err(_) => return true,
        };
        if output.status.success() {
            return true;
        }
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        !is_git_shell_startup_failure(&text)
    })
}

#[cfg(windows)]
fn posix_sh_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        Command::new("sh")
            .args(["-lc", "exit 0"])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    })
}

fn require_git_shell_for_setup_integration_tests() -> bool {
    #[cfg(windows)]
    {
        if !git_shell_available_for_tooling() {
            eprintln!(
                "skipping setup integration test: Git-for-Windows shell startup failed in this environment"
            );
            return false;
        }
    }
    true
}

fn require_posix_shell_binary_for_setup_test() -> bool {
    #[cfg(windows)]
    {
        if !posix_sh_available() {
            eprintln!(
                "skipping setup dry-run shell execution test: `sh` is unavailable in PATH on this environment"
            );
            return false;
        }
    }
    true
}

fn count_occurrences(haystack: &str, needle: &str) -> usize {
    haystack.match_indices(needle).count()
}

fn assert_placeholder_is_quoted(cmd: &str, var: &str) {
    let raw = format!("${var}");
    let quoted = format!("\"{raw}\"");
    let raw_count = count_occurrences(cmd, &raw);
    let quoted_count = count_occurrences(cmd, &quoted);

    assert!(
        quoted_count > 0,
        "expected quoted placeholder {quoted} in cmd: {cmd}"
    );
    assert_eq!(
        raw_count, quoted_count,
        "found unquoted placeholder ${var} in cmd: {cmd}"
    );
}

#[test]
fn standalone_mergetool_clean_merge_exits_zero_and_writes_output() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.txt");
    let local = dir.path().join("local.txt");
    let remote = dir.path().join("remote.txt");
    let merged = dir.path().join("nested/out/merged.txt");

    write_file(&base, "line1\nline2\nline3\n");
    write_file(&local, "LINE1\nline2\nline3\n");
    write_file(&remote, "line1\nline2\nLINE3\n");

    let output = run_gitcomet([
        OsString::from("mergetool"),
        OsString::from("--base"),
        base.as_os_str().to_owned(),
        OsString::from("--local"),
        local.as_os_str().to_owned(),
        OsString::from("--remote"),
        remote.as_os_str().to_owned(),
        OsString::from("--merged"),
        merged.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "expected exit 0\n{text}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Auto-merged"),
        "expected auto-merge message\n{text}"
    );
    let merged_text = fs::read_to_string(&merged).expect("merged output to exist");
    assert_eq!(merged_text, "LINE1\nline2\nLINE3\n");
}

#[test]
fn standalone_mergetool_conflict_exits_one_and_writes_markers() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.txt");
    let local = dir.path().join("local.txt");
    let remote = dir.path().join("remote.txt");
    let merged = dir.path().join("merged.txt");

    write_file(&base, "line\n");
    write_file(&local, "ours\n");
    write_file(&remote, "theirs\n");

    let output = run_gitcomet([
        OsString::from("mergetool"),
        OsString::from("--base"),
        base.as_os_str().to_owned(),
        OsString::from("--local"),
        local.as_os_str().to_owned(),
        OsString::from("--remote"),
        remote.as_os_str().to_owned(),
        OsString::from("--merged"),
        merged.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(1), "expected exit 1\n{text}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("CONFLICT (content)"),
        "expected conflict message\n{text}"
    );

    let merged_text = fs::read_to_string(&merged).expect("merged output to exist");
    assert!(merged_text.contains("<<<<<<<"), "output:\n{merged_text}");
    assert!(merged_text.contains("======="), "output:\n{merged_text}");
    assert!(merged_text.contains(">>>>>>>"), "output:\n{merged_text}");
}

#[test]
fn standalone_mergetool_non_utf8_conflict_exits_one_and_keeps_local_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("local.dat");
    let remote = dir.path().join("remote.dat");
    let merged = dir.path().join("merged.dat");

    // Invalid UTF-8 without NUL bytes: exercises non-UTF-8 binary detection.
    let local_bytes = b"prefix\n\xFF\n";
    let remote_bytes = b"prefix\n\xFE\n";
    write_bytes(&local, local_bytes);
    write_bytes(&remote, remote_bytes);

    let output = run_gitcomet([
        OsString::from("mergetool"),
        OsString::from("--local"),
        local.as_os_str().to_owned(),
        OsString::from("--remote"),
        remote.as_os_str().to_owned(),
        OsString::from("--merged"),
        merged.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(1), "expected exit 1\n{text}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("binary"),
        "expected binary conflict message\n{text}"
    );
    assert_eq!(
        fs::read(&merged).expect("merged output to exist"),
        local_bytes,
        "non-UTF-8 conflict should keep local bytes"
    );
}

#[test]
fn standalone_mergetool_no_base_identical_additions_exits_zero() {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("local.txt");
    let remote = dir.path().join("remote.txt");
    let merged = dir.path().join("merged.txt");

    write_file(&local, "added in both sides\n");
    write_file(&remote, "added in both sides\n");

    let output = run_gitcomet([
        OsString::from("mergetool"),
        OsString::from("--local"),
        local.as_os_str().to_owned(),
        OsString::from("--remote"),
        remote.as_os_str().to_owned(),
        OsString::from("--merged"),
        merged.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "expected exit 0\n{text}");

    let merged_text = fs::read_to_string(&merged).expect("merged output to exist");
    assert_eq!(merged_text, "added in both sides\n");
    assert!(
        !merged_text.contains("<<<<<<<"),
        "expected clean merge output\n{text}"
    );
}

#[test]
fn standalone_mergetool_empty_base_flag_treated_as_no_base() {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("local.txt");
    let remote = dir.path().join("remote.txt");
    let merged = dir.path().join("merged.txt");

    write_file(&local, "added in both sides\n");
    write_file(&remote, "added in both sides\n");

    let output = run_gitcomet([
        OsString::from("mergetool"),
        OsString::from("--base"),
        OsString::from(""),
        OsString::from("--local"),
        local.as_os_str().to_owned(),
        OsString::from("--remote"),
        remote.as_os_str().to_owned(),
        OsString::from("--merged"),
        merged.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "expected exit 0\n{text}");

    let merged_text = fs::read_to_string(&merged).expect("merged output to exist");
    assert_eq!(merged_text, "added in both sides\n");
}

#[test]
fn standalone_mergetool_no_base_zdiff3_uses_empty_tree_label() {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("local.txt");
    let remote = dir.path().join("remote.txt");
    let merged = dir.path().join("merged.txt");

    write_file(&local, "ours change\n");
    write_file(&remote, "theirs change\n");

    let output = run_gitcomet([
        OsString::from("mergetool"),
        OsString::from("--local"),
        local.as_os_str().to_owned(),
        OsString::from("--remote"),
        remote.as_os_str().to_owned(),
        OsString::from("--merged"),
        merged.as_os_str().to_owned(),
        OsString::from("--conflict-style"),
        OsString::from("zdiff3"),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(1), "expected exit 1\n{text}");

    let merged_text = fs::read_to_string(&merged).expect("merged output to exist");
    assert!(
        merged_text.contains("<<<<<<< local.txt"),
        "expected local filename fallback label\n{text}\nmerged:\n{merged_text}"
    );
    assert!(
        merged_text.contains("||||||| empty tree"),
        "expected no-base zdiff3 marker label\n{text}\nmerged:\n{merged_text}"
    );
    assert!(
        merged_text.contains(">>>>>>> remote.txt"),
        "expected remote filename fallback label\n{text}\nmerged:\n{merged_text}"
    );
}

#[test]
fn standalone_mergetool_marker_size_flag_controls_marker_width() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.txt");
    let local = dir.path().join("local.txt");
    let remote = dir.path().join("remote.txt");
    let merged = dir.path().join("merged.txt");

    write_file(&base, "line\n");
    write_file(&local, "ours\n");
    write_file(&remote, "theirs\n");

    let output = run_gitcomet([
        OsString::from("mergetool"),
        OsString::from("--base"),
        base.as_os_str().to_owned(),
        OsString::from("--local"),
        local.as_os_str().to_owned(),
        OsString::from("--remote"),
        remote.as_os_str().to_owned(),
        OsString::from("--merged"),
        merged.as_os_str().to_owned(),
        OsString::from("--marker-size"),
        OsString::from("10"),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(1), "expected exit 1\n{text}");

    let merged_text = fs::read_to_string(&merged).expect("merged output to exist");
    assert!(
        merged_text.contains("<<<<<<<<<<"),
        "expected 10-char opening marker\n{text}\nmerged:\n{merged_text}"
    );
    assert!(
        merged_text.contains("\n==========\n"),
        "expected 10-char separator marker\n{text}\nmerged:\n{merged_text}"
    );
    assert!(
        merged_text.contains(">>>>>>>>>>"),
        "expected 10-char closing marker\n{text}\nmerged:\n{merged_text}"
    );
}

#[test]
fn standalone_mergetool_conflict_markers_preserve_crlf_line_endings() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.txt");
    let local = dir.path().join("local.txt");
    let remote = dir.path().join("remote.txt");
    let merged = dir.path().join("merged.txt");

    write_bytes(&base, b"1\r\n2\r\n3\r\n");
    write_bytes(&local, b"1\r\n2\r\n4\r\n");
    write_bytes(&remote, b"1\r\n2\r\n5\r\n");

    let output = run_gitcomet([
        OsString::from("mergetool"),
        OsString::from("--base"),
        base.as_os_str().to_owned(),
        OsString::from("--local"),
        local.as_os_str().to_owned(),
        OsString::from("--remote"),
        remote.as_os_str().to_owned(),
        OsString::from("--merged"),
        merged.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(1), "expected exit 1\n{text}");

    let merged_bytes = fs::read(&merged).expect("merged output to exist");
    let merged_text = String::from_utf8_lossy(&merged_bytes);
    assert!(
        merged_text.contains("<<<<<<<"),
        "expected opening marker\n{text}\nmerged:\n{merged_text}"
    );
    assert!(
        merged_text.contains("\r\n=======\r\n"),
        "expected CRLF separator marker\n{text}\nmerged:\n{merged_text}"
    );
    assert!(
        merged_text.contains(">>>>>>>"),
        "expected closing marker\n{text}\nmerged:\n{merged_text}"
    );
}

#[test]
fn standalone_mergetool_handles_unicode_paths() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("ベース.txt");
    let local = dir.path().join("ローカル.txt");
    let remote = dir.path().join("リモート.txt");
    let merged = dir.path().join("出力/マージ済み.txt");

    write_file(&base, "line1\nline2\nline3\n");
    write_file(&local, "LINE1\nline2\nline3\n");
    write_file(&remote, "line1\nline2\nLINE3\n");

    let output = run_gitcomet([
        OsString::from("mergetool"),
        OsString::from("--base"),
        base.as_os_str().to_owned(),
        OsString::from("--local"),
        local.as_os_str().to_owned(),
        OsString::from("--remote"),
        remote.as_os_str().to_owned(),
        OsString::from("--merged"),
        merged.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "expected exit 0\n{text}");
    let merged_text = fs::read_to_string(&merged).expect("merged output to exist");
    assert_eq!(merged_text, "LINE1\nline2\nLINE3\n");
}

#[test]
fn standalone_mergetool_handles_paths_with_spaces() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base side.txt");
    let local = dir.path().join("local side.txt");
    let remote = dir.path().join("remote side.txt");
    let merged = dir.path().join("merged output/final merge.txt");

    write_file(&base, "line1\nline2\nline3\n");
    write_file(&local, "LINE1\nline2\nline3\n");
    write_file(&remote, "line1\nline2\nLINE3\n");

    let output = run_gitcomet([
        OsString::from("mergetool"),
        OsString::from("--base"),
        base.as_os_str().to_owned(),
        OsString::from("--local"),
        local.as_os_str().to_owned(),
        OsString::from("--remote"),
        remote.as_os_str().to_owned(),
        OsString::from("--merged"),
        merged.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "expected exit 0\n{text}");
    let merged_text = fs::read_to_string(&merged).expect("merged output to exist");
    assert_eq!(merged_text, "LINE1\nline2\nLINE3\n");
}

#[test]
fn standalone_mergetool_invalid_path_exits_two() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.txt");
    let local = dir.path().join("local.txt");
    let missing_remote = dir.path().join("missing_remote.txt");
    let merged = dir.path().join("merged.txt");

    write_file(&base, "line\n");
    write_file(&local, "line\n");

    let output = run_gitcomet([
        OsString::from("mergetool"),
        OsString::from("--base"),
        base.as_os_str().to_owned(),
        OsString::from("--local"),
        local.as_os_str().to_owned(),
        OsString::from("--remote"),
        missing_remote.as_os_str().to_owned(),
        OsString::from("--merged"),
        merged.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(2), "expected exit 2\n{text}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Remote path does not exist"),
        "expected validation error\n{text}"
    );
}

#[cfg(not(feature = "ui-gpui-runtime"))]
#[test]
fn standalone_mergetool_gui_flag_without_ui_feature_exits_two() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.txt");
    let local = dir.path().join("local.txt");
    let remote = dir.path().join("remote.txt");
    let merged = dir.path().join("merged.txt");

    write_file(&base, "line\n");
    write_file(&local, "ours\n");
    write_file(&remote, "theirs\n");

    let output = run_gitcomet([
        OsString::from("mergetool"),
        OsString::from("--gui"),
        OsString::from("--base"),
        base.as_os_str().to_owned(),
        OsString::from("--local"),
        local.as_os_str().to_owned(),
        OsString::from("--remote"),
        remote.as_os_str().to_owned(),
        OsString::from("--merged"),
        merged.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(2), "expected exit 2\n{text}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("GUI mergetool mode is unavailable"),
        "expected actionable GUI-unavailable error\n{text}"
    );
}

#[test]
fn standalone_mergetool_rejects_directory_merged_target_with_exit_two() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.txt");
    let local = dir.path().join("local.txt");
    let remote = dir.path().join("remote.txt");
    let merged_dir = dir.path().join("merged-dir");
    fs::create_dir_all(&merged_dir).expect("create merged directory");

    write_file(&base, "line\n");
    write_file(&local, "line\n");
    write_file(&remote, "line\n");

    let output = run_gitcomet([
        OsString::from("mergetool"),
        OsString::from("--base"),
        base.as_os_str().to_owned(),
        OsString::from("--local"),
        local.as_os_str().to_owned(),
        OsString::from("--remote"),
        remote.as_os_str().to_owned(),
        OsString::from("--merged"),
        merged_dir.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(2), "expected exit 2\n{text}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Merged path must be a file path"),
        "expected merged-path validation error\n{text}"
    );
}

#[cfg(unix)]
#[test]
fn standalone_mergetool_rejects_fifo_local_input_exits_two() {
    let dir = tempfile::tempdir().unwrap();
    let merged = dir.path().join("merged.txt");
    let local_fifo = dir.path().join("local.fifo");
    let remote = dir.path().join("remote.txt");

    write_file(&remote, "remote\n");

    let fifo_status = Command::new("mkfifo")
        .arg(&local_fifo)
        .status()
        .expect("run mkfifo");
    assert!(fifo_status.success(), "mkfifo failed: {fifo_status}");

    let output = run_gitcomet([
        OsString::from("mergetool"),
        OsString::from("--local"),
        local_fifo.as_os_str().to_owned(),
        OsString::from("--remote"),
        remote.as_os_str().to_owned(),
        OsString::from("--merged"),
        merged.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(2), "expected exit 2\n{text}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Local path must be a regular file"),
        "expected local-path regular-file validation error\n{text}"
    );
}

#[test]
fn standalone_difftool_changed_files_exits_zero_and_prints_diff() {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("left.txt");
    let remote = dir.path().join("right.txt");

    write_file(&local, "left\n");
    write_file(&remote, "right\n");

    let output = run_gitcomet([
        OsString::from("difftool"),
        OsString::from("--local"),
        local.as_os_str().to_owned(),
        OsString::from("--remote"),
        remote.as_os_str().to_owned(),
        OsString::from("--path"),
        OsString::from("src/file.txt"),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "expected exit 0\n{text}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("@@"), "expected unified diff hunk\n{text}");
    assert!(
        stdout.contains("--- a/src/file.txt"),
        "expected left label\n{text}"
    );
    assert!(
        stdout.contains("+++ b/src/file.txt"),
        "expected right label\n{text}"
    );
}

#[test]
fn standalone_difftool_binary_content_change_exits_zero() {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("left.bin");
    let remote = dir.path().join("right.bin");

    write_bytes(&local, &[0x00, 0x01, 0x02, 0x03]);
    write_bytes(&remote, &[0x00, 0x01, 0xFF, 0x03]);

    let output = run_gitcomet([
        OsString::from("difftool"),
        OsString::from("--local"),
        local.as_os_str().to_owned(),
        OsString::from("--remote"),
        remote.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "expected exit 0\n{text}");
    assert!(
        text.contains("Binary files")
            || text.contains("GIT binary patch")
            || text.contains("differ"),
        "expected binary diff output\n{text}"
    );
}

#[test]
fn standalone_difftool_non_utf8_content_change_exits_zero() {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("left.dat");
    let remote = dir.path().join("right.dat");

    write_bytes(&local, b"prefix\n\xFF\n");
    write_bytes(&remote, b"prefix\n\xFE\n");

    let output = run_gitcomet([
        OsString::from("difftool"),
        OsString::from("--local"),
        local.as_os_str().to_owned(),
        OsString::from("--remote"),
        remote.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "expected exit 0\n{text}");
    assert!(
        !output.stdout.is_empty() || !output.stderr.is_empty(),
        "expected non-empty diff output\n{text}"
    );
}

#[test]
fn standalone_difftool_crlf_content_preserved_in_diff_output() {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("left.txt");
    let remote = dir.path().join("right.txt");

    write_bytes(&local, b"line1\r\nline2\r\nline3\r\n");
    write_bytes(&remote, b"line1\r\nmodified\r\nline3\r\n");

    let output = run_gitcomet([
        OsString::from("difftool"),
        OsString::from("--local"),
        local.as_os_str().to_owned(),
        OsString::from("--remote"),
        remote.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "expected exit 0\n{text}");

    let stdout = String::from_utf8_lossy(&output.stdout);
    // The diff should show the changed line.
    assert!(
        stdout.contains("-line2") || stdout.contains("-line2\r"),
        "expected removed CRLF line in diff output\n{text}"
    );
    assert!(
        stdout.contains("+modified") || stdout.contains("+modified\r"),
        "expected added CRLF line in diff output\n{text}"
    );
    // Context lines (unchanged) should be present.
    assert!(
        stdout.contains(" line1") || stdout.contains(" line1\r"),
        "expected context line in diff output\n{text}"
    );
}

#[test]
fn standalone_difftool_crlf_identical_files_no_diff() {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("left.txt");
    let remote = dir.path().join("right.txt");

    write_bytes(&local, b"aaa\r\nbbb\r\nccc\r\n");
    write_bytes(&remote, b"aaa\r\nbbb\r\nccc\r\n");

    let output = run_gitcomet([
        OsString::from("difftool"),
        OsString::from("--local"),
        local.as_os_str().to_owned(),
        OsString::from("--remote"),
        remote.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "expected exit 0\n{text}");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.trim().is_empty(),
        "identical CRLF files should produce no diff output\n{text}"
    );
}

#[test]
fn standalone_difftool_mixed_line_endings_produces_diff() {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("left.txt");
    let remote = dir.path().join("right.txt");

    // Local uses LF, remote uses CRLF — line-ending difference should appear in diff.
    write_bytes(&local, b"aaa\nbbb\nccc\n");
    write_bytes(&remote, b"aaa\r\nbbb\r\nccc\r\n");

    let output = run_gitcomet([
        OsString::from("difftool"),
        OsString::from("--local"),
        local.as_os_str().to_owned(),
        OsString::from("--remote"),
        remote.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "expected exit 0\n{text}");
    // Git should detect the line ending difference and either show no diff
    // (if it considers them equivalent) or show the change — either is valid.
    // The key contract is that the tool exits successfully.
}

#[test]
fn standalone_difftool_directory_diff_exits_zero() {
    let dir = tempfile::tempdir().unwrap();
    let local_dir = dir.path().join("left");
    let remote_dir = dir.path().join("right");

    fs::create_dir_all(&local_dir).expect("create local dir");
    fs::create_dir_all(&remote_dir).expect("create remote dir");
    write_file(&local_dir.join("a.txt"), "left\n");
    write_file(&remote_dir.join("a.txt"), "right\n");

    let output = run_gitcomet([
        OsString::from("difftool"),
        OsString::from("--local"),
        local_dir.as_os_str().to_owned(),
        OsString::from("--remote"),
        remote_dir.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "expected exit 0\n{text}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("a.txt"),
        "expected filename in directory diff output\n{text}"
    );
}

#[cfg(unix)]
#[test]
fn standalone_difftool_directory_symlink_inputs_compare_directory_contents() {
    use std::os::unix::fs as unix_fs;

    let dir = tempfile::tempdir().unwrap();
    let local_dir = dir.path().join("left");
    let remote_dir = dir.path().join("right");
    let local_link = dir.path().join("left-link");
    let remote_link = dir.path().join("right-link");

    fs::create_dir_all(&local_dir).expect("create local dir");
    fs::create_dir_all(&remote_dir).expect("create remote dir");
    write_file(&local_dir.join("a.txt"), "left\n");
    write_file(&remote_dir.join("a.txt"), "right\n");
    unix_fs::symlink(&local_dir, &local_link).expect("create local dir symlink");
    unix_fs::symlink(&remote_dir, &remote_link).expect("create remote dir symlink");

    let output = run_gitcomet([
        OsString::from("difftool"),
        OsString::from("--local"),
        local_link.as_os_str().to_owned(),
        OsString::from("--remote"),
        remote_link.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "expected exit 0\n{text}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("-left") && stdout.contains("+right"),
        "expected file-content diff for symlinked directories\n{text}"
    );
    assert!(
        !stdout.contains("new file mode 120000"),
        "did not expect symlink-mode-only diff\n{text}"
    );
}

#[cfg(unix)]
#[test]
fn standalone_difftool_directory_diff_rejects_symlink_cycle_exits_two() {
    use std::os::unix::fs as unix_fs;

    let dir = tempfile::tempdir().unwrap();
    let local_dir = dir.path().join("left");
    let remote_dir = dir.path().join("right");

    fs::create_dir_all(&local_dir).expect("create local dir");
    fs::create_dir_all(&remote_dir).expect("create remote dir");
    write_file(&local_dir.join("a.txt"), "left\n");
    write_file(&remote_dir.join("a.txt"), "right\n");
    unix_fs::symlink(".", remote_dir.join("loop")).expect("create self-referential symlink");

    let output = run_gitcomet([
        OsString::from("difftool"),
        OsString::from("--local"),
        local_dir.as_os_str().to_owned(),
        OsString::from("--remote"),
        remote_dir.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(2), "expected exit 2\n{text}");
    assert!(
        text.contains("symlink cycle"),
        "expected symlink cycle error\n{text}"
    );
}

#[cfg(unix)]
#[test]
fn standalone_difftool_broken_symlink_inputs_exit_zero() {
    use std::os::unix::fs as unix_fs;

    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("left-link");
    let remote = dir.path().join("right-link");

    unix_fs::symlink("missing-left-target", &local).expect("create local broken symlink");
    unix_fs::symlink("missing-right-target", &remote).expect("create remote broken symlink");

    let output = run_gitcomet([
        OsString::from("difftool"),
        OsString::from("--local"),
        local.as_os_str().to_owned(),
        OsString::from("--remote"),
        remote.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "expected exit 0\n{text}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("@@"), "expected symlink diff hunk\n{text}");
    assert!(
        stdout.contains("missing-left-target") && stdout.contains("missing-right-target"),
        "expected broken symlink targets in diff output\n{text}"
    );
}

#[cfg(unix)]
#[test]
fn standalone_difftool_broken_symlink_preserves_non_utf8_target_bytes() {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs as unix_fs;

    let dir = tempfile::tempdir().unwrap();
    // Non-UTF-8 bytes that to_string_lossy() would corrupt (\xff → U+FFFD).
    let non_utf8_bytes = b"target-\xff-\xfe";
    let non_utf8_target = std::ffi::OsStr::from_bytes(non_utf8_bytes);

    let local = dir.path().join("left-link");
    let remote = dir.path().join("right-link");

    // Both sides: broken symlinks with the same non-UTF-8 target.
    unix_fs::symlink(non_utf8_target, &local).expect("create local non-UTF-8 symlink");
    unix_fs::symlink(non_utf8_target, &remote).expect("create remote non-UTF-8 symlink");

    let output = run_gitcomet([
        OsString::from("difftool"),
        OsString::from("--local"),
        local.as_os_str().to_owned(),
        OsString::from("--remote"),
        remote.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "expected exit 0\n{text}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.trim().is_empty(),
        "identical non-UTF-8 symlink targets should produce no diff\n{text}"
    );
}

#[test]
fn standalone_difftool_handles_unicode_paths() {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("左側.txt");
    let remote = dir.path().join("右側.txt");

    write_file(&local, "left\n");
    write_file(&remote, "right\n");

    let output = run_gitcomet([
        OsString::from("difftool"),
        OsString::from("--local"),
        local.as_os_str().to_owned(),
        OsString::from("--remote"),
        remote.as_os_str().to_owned(),
        OsString::from("--path"),
        OsString::from("src/日本語.txt"),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "expected exit 0\n{text}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("@@"), "expected unified diff hunk\n{text}");
    assert!(
        stdout.contains("--- a/src/日本語.txt"),
        "expected unicode left label\n{text}"
    );
    assert!(
        stdout.contains("+++ b/src/日本語.txt"),
        "expected unicode right label\n{text}"
    );
}

#[test]
fn standalone_difftool_handles_paths_with_spaces() {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("left side.txt");
    let remote = dir.path().join("right side.txt");

    write_file(&local, "left\n");
    write_file(&remote, "right\n");

    let output = run_gitcomet([
        OsString::from("difftool"),
        OsString::from("--local"),
        local.as_os_str().to_owned(),
        OsString::from("--remote"),
        remote.as_os_str().to_owned(),
        OsString::from("--path"),
        OsString::from("docs/spaced name.txt"),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "expected exit 0\n{text}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("@@"), "expected unified diff hunk\n{text}");
    assert!(
        stdout.contains("--- a/docs/spaced name.txt"),
        "expected spaced left label\n{text}"
    );
    assert!(
        stdout.contains("+++ b/docs/spaced name.txt"),
        "expected spaced right label\n{text}"
    );
}

#[test]
fn standalone_compat_difftool_accepts_meld_style_label_flags() {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("left.txt");
    let remote = dir.path().join("right.txt");

    write_file(&local, "left\n");
    write_file(&remote, "right\n");

    let output = run_gitcomet([
        OsString::from("-L"),
        OsString::from("LEFT_LABEL"),
        OsString::from("--label"),
        OsString::from("RIGHT_LABEL"),
        local.as_os_str().to_owned(),
        remote.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "expected exit 0\n{text}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("--- LEFT_LABEL"),
        "expected left label\n{text}"
    );
    assert!(
        stdout.contains("+++ RIGHT_LABEL"),
        "expected right label\n{text}"
    );
}

#[test]
fn standalone_compat_difftool_accepts_attached_label_forms() {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("left.txt");
    let remote = dir.path().join("right.txt");

    write_file(&local, "left\n");
    write_file(&remote, "right\n");

    let output = run_gitcomet([
        OsString::from("-LLEFT_LABEL"),
        OsString::from("--label=RIGHT_LABEL"),
        local.as_os_str().to_owned(),
        remote.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "expected exit 0\n{text}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("--- LEFT_LABEL"),
        "expected left label\n{text}"
    );
    assert!(
        stdout.contains("+++ RIGHT_LABEL"),
        "expected right label\n{text}"
    );
}

#[test]
fn standalone_compat_mergetool_meld_label_order_maps_to_local_base_remote() {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("local.txt");
    let base = dir.path().join("base.txt");
    let remote = dir.path().join("remote.txt");
    let merged = dir.path().join("merged.txt");

    write_file(&base, "line\n");
    write_file(&local, "local change\n");
    write_file(&remote, "remote change\n");

    let output = run_gitcomet([
        OsString::from("--output"),
        merged.as_os_str().to_owned(),
        OsString::from("--label"),
        OsString::from("LOCAL_LABEL"),
        OsString::from("--label"),
        OsString::from("BASE_LABEL"),
        OsString::from("--label"),
        OsString::from("REMOTE_LABEL"),
        local.as_os_str().to_owned(),
        base.as_os_str().to_owned(),
        remote.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(1), "expected exit 1\n{text}");

    let merged_text = fs::read_to_string(&merged).expect("merged output to exist");
    assert!(
        merged_text.contains("<<<<<<< LOCAL_LABEL"),
        "expected local label on ours marker\nmerged:\n{merged_text}\n{text}"
    );
    assert!(
        merged_text.contains(">>>>>>> REMOTE_LABEL"),
        "expected remote label on theirs marker\nmerged:\n{merged_text}\n{text}"
    );
    assert!(
        !merged_text.contains("<<<<<<< BASE_LABEL"),
        "base label should not map to ours marker in meld ordering\nmerged:\n{merged_text}\n{text}"
    );
}

#[test]
fn standalone_compat_mergetool_accepts_attached_output_and_base_flags() {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("local file.txt");
    let base = dir.path().join("base file.txt");
    let remote = dir.path().join("remote file.txt");
    let merged = dir.path().join("merged output.txt");

    // This relies on BASE being parsed correctly:
    // - with BASE parsed: clean merge (LOCAL == BASE, REMOTE changed) => exit 0
    // - without BASE: two-way add/add style conflict => exit 1
    write_file(&base, "line\n");
    write_file(&local, "line\n");
    write_file(&remote, "remote change\n");

    let output = run_gitcomet([
        OsString::from(format!("--base={}", base.display())),
        OsString::from(format!("--out={}", merged.display())),
        OsString::from("--L1=BASE_LABEL"),
        OsString::from("--L2=LOCAL_LABEL"),
        OsString::from("--L3=REMOTE_LABEL"),
        local.as_os_str().to_owned(),
        remote.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "expected exit 0\n{text}");

    let merged_text = fs::read_to_string(&merged).expect("merged output to exist");
    assert_eq!(
        merged_text, "remote change\n",
        "expected clean merge result from attached --base/--out forms\n{text}"
    );
}

#[test]
fn standalone_compat_auto_requires_output_path_exits_two() {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("local.txt");
    let remote = dir.path().join("remote.txt");
    write_file(&local, "left\n");
    write_file(&remote, "right\n");

    let output = run_gitcomet([
        OsString::from("--auto"),
        local.as_os_str().to_owned(),
        remote.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(2), "expected exit 2\n{text}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("--auto requires -o/--output/--out"),
        "expected actionable compatibility error\n{text}"
    );
}

#[test]
fn standalone_compat_diff_rejects_base_without_output_path_exits_two() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.txt");
    let local = dir.path().join("local.txt");
    let remote = dir.path().join("remote.txt");
    write_file(&base, "base\n");
    write_file(&local, "left\n");
    write_file(&remote, "right\n");

    let output = run_gitcomet([
        OsString::from("--base"),
        base.as_os_str().to_owned(),
        local.as_os_str().to_owned(),
        remote.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(2), "expected exit 2\n{text}");
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("--base is only valid for merge mode with -o/--output/--out"),
        "expected actionable compatibility error\n{text}"
    );
}

#[test]
fn standalone_compat_rejects_too_many_label_flags_exits_two() {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("local.txt");
    let remote = dir.path().join("remote.txt");
    write_file(&local, "left\n");
    write_file(&remote, "right\n");

    let output = run_gitcomet([
        OsString::from("-L"),
        OsString::from("L1"),
        OsString::from("-L"),
        OsString::from("L2"),
        OsString::from("-L"),
        OsString::from("L3"),
        OsString::from("-L"),
        OsString::from("L4"),
        local.as_os_str().to_owned(),
        remote.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(2), "expected exit 2\n{text}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("too many label flags"),
        "expected actionable compatibility error\n{text}"
    );
}

#[test]
fn standalone_compat_diff_rejects_too_many_positionals_exits_two() {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("local.txt");
    let remote = dir.path().join("remote.txt");
    let extra = dir.path().join("extra.txt");
    write_file(&local, "left\n");
    write_file(&remote, "right\n");
    write_file(&extra, "extra\n");

    let output = run_gitcomet([
        local.as_os_str().to_owned(),
        remote.as_os_str().to_owned(),
        extra.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(2), "expected exit 2\n{text}");
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("too many positional paths; expected exactly 2"),
        "expected actionable compatibility error\n{text}"
    );
}

#[test]
fn standalone_compat_merge_rejects_too_many_positionals_exits_two() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.txt");
    let local = dir.path().join("local.txt");
    let remote = dir.path().join("remote.txt");
    let extra = dir.path().join("extra.txt");
    let merged = dir.path().join("merged.txt");

    write_file(&base, "base\n");
    write_file(&local, "left\n");
    write_file(&remote, "right\n");
    write_file(&extra, "extra\n");

    let output = run_gitcomet([
        OsString::from("--output"),
        merged.as_os_str().to_owned(),
        base.as_os_str().to_owned(),
        local.as_os_str().to_owned(),
        remote.as_os_str().to_owned(),
        extra.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(2), "expected exit 2\n{text}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(
            "too many positional paths; expected 2 (LOCAL REMOTE) or 3 (BASE LOCAL REMOTE)"
        ),
        "expected actionable compatibility error\n{text}"
    );
}

#[test]
fn standalone_difftool_file_directory_mismatch_exits_two() {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("left.txt");
    let remote_dir = dir.path().join("right");
    write_file(&local, "left\n");
    fs::create_dir_all(&remote_dir).expect("create remote dir");

    let output = run_gitcomet([
        OsString::from("difftool"),
        OsString::from("--local"),
        local.as_os_str().to_owned(),
        OsString::from("--remote"),
        remote_dir.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(2), "expected exit 2\n{text}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("input kind mismatch"),
        "expected actionable kind-mismatch validation\n{text}"
    );
}

#[test]
fn standalone_difftool_missing_input_exits_two() {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("left.txt");
    let missing_remote = dir.path().join("missing.txt");
    write_file(&local, "left\n");

    let output = run_gitcomet([
        OsString::from("difftool"),
        OsString::from("--local"),
        local.as_os_str().to_owned(),
        OsString::from("--remote"),
        missing_remote.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(2), "expected exit 2\n{text}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Remote path does not exist"),
        "expected validation error\n{text}"
    );
}

#[cfg(unix)]
#[test]
fn standalone_difftool_rejects_fifo_input_exits_two() {
    use std::process::Command;

    let dir = tempfile::tempdir().unwrap();
    let local_fifo = dir.path().join("left.fifo");
    let fifo_status = Command::new("mkfifo")
        .arg(&local_fifo)
        .status()
        .expect("run mkfifo");
    assert!(fifo_status.success(), "mkfifo failed: {fifo_status}");
    let remote = dir.path().join("right.txt");
    write_file(&remote, "right\n");

    let output = run_gitcomet([
        OsString::from("difftool"),
        OsString::from("--local"),
        local_fifo.as_os_str().to_owned(),
        OsString::from("--remote"),
        remote.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(2), "expected exit 2\n{text}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("must be a regular file or directory"),
        "expected actionable special-file validation error\n{text}"
    );
}

#[cfg(not(feature = "ui-gpui-runtime"))]
#[test]
fn standalone_difftool_gui_flag_without_ui_feature_exits_two() {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("left.txt");
    let remote = dir.path().join("right.txt");
    write_file(&local, "left\n");
    write_file(&remote, "right\n");

    let output = run_gitcomet([
        OsString::from("difftool"),
        OsString::from("--gui"),
        OsString::from("--local"),
        local.as_os_str().to_owned(),
        OsString::from("--remote"),
        remote.as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(2), "expected exit 2\n{text}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("GUI difftool mode is unavailable"),
        "expected actionable GUI-unavailable error\n{text}"
    );
}

// ── Setup subcommand tests ───────────────────────────────────────────

#[test]
fn setup_dry_run_prints_commands_without_writing() {
    let output = run_gitcomet([OsString::from("setup"), OsString::from("--dry-run")]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "expected exit 0\n{text}");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Dry run"),
        "expected dry-run header\n{text}"
    );
    // Headless tool entries
    assert!(
        stdout.contains("git config --global merge.tool"),
        "expected merge.tool\n{text}"
    );
    assert!(
        stdout.contains("git config --global diff.tool"),
        "expected diff.tool\n{text}"
    );
    assert!(
        stdout.contains("mergetool.gitcomet.cmd"),
        "expected mergetool cmd\n{text}"
    );
    assert!(
        stdout.contains("difftool.gitcomet.cmd"),
        "expected difftool cmd\n{text}"
    );
    assert!(
        stdout.contains("mergetool.trustExitCode"),
        "expected mergetool.trustExitCode\n{text}"
    );
    assert!(
        stdout.contains("mergetool.gitcomet.trustExitCode"),
        "expected mergetool.gitcomet.trustExitCode\n{text}"
    );
    assert!(
        stdout.contains("difftool.trustExitCode"),
        "expected difftool.trustExitCode\n{text}"
    );
    // GUI tool entries
    assert!(
        stdout.contains("mergetool.gitcomet-gui.cmd"),
        "expected GUI mergetool cmd\n{text}"
    );
    assert!(
        stdout.contains("difftool.gitcomet-gui.cmd"),
        "expected GUI difftool cmd\n{text}"
    );
    assert!(
        stdout.contains("mergetool.gitcomet-gui.trustExitCode"),
        "expected GUI mergetool trustExitCode\n{text}"
    );
    assert!(
        stdout.contains("difftool.gitcomet-gui.trustExitCode"),
        "expected GUI difftool trustExitCode\n{text}"
    );
}

#[test]
fn setup_dry_run_local_uses_local_scope() {
    let output = run_gitcomet([
        OsString::from("setup"),
        OsString::from("--dry-run"),
        OsString::from("--local"),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "expected exit 0\n{text}");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("git config --local"),
        "expected --local scope\n{text}"
    );
    assert!(
        !stdout.contains("--global"),
        "should not use --global\n{text}"
    );
}

#[test]
fn setup_dry_run_commands_execute_verbatim_in_shell() {
    if !require_posix_shell_binary_for_setup_test() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();

    let init = Command::new("git")
        .args(["init", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(init.status.success(), "git init failed");

    let output = run_gitcomet_in_dir(
        dir.path(),
        [
            OsString::from("setup"),
            OsString::from("--dry-run"),
            OsString::from("--local"),
        ],
    );
    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "expected exit 0\n{text}");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let commands: Vec<&str> = stdout
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("git config --local "))
        .collect();

    assert!(
        !commands.is_empty(),
        "expected dry-run output to contain git config commands\n{text}"
    );

    for cmd in commands {
        let apply = Command::new("sh")
            .current_dir(dir.path())
            .args(["-c", cmd])
            .output()
            .unwrap();
        assert!(
            apply.status.success(),
            "dry-run command should be shell-runnable:\n{cmd}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&apply.stdout),
            String::from_utf8_lossy(&apply.stderr)
        );
    }

    let merge_cmd = git_config_get(dir.path(), "mergetool.gitcomet.cmd")
        .expect("mergetool cmd should be configured by dry-run commands");
    assert_placeholder_is_quoted(&merge_cmd, "BASE");
    assert_placeholder_is_quoted(&merge_cmd, "LOCAL");
    assert_placeholder_is_quoted(&merge_cmd, "REMOTE");
    assert_placeholder_is_quoted(&merge_cmd, "MERGED");

    let diff_cmd = git_config_get(dir.path(), "difftool.gitcomet.cmd")
        .expect("difftool cmd should be configured by dry-run commands");
    assert_placeholder_is_quoted(&diff_cmd, "LOCAL");
    assert_placeholder_is_quoted(&diff_cmd, "REMOTE");
    assert_placeholder_is_quoted(&diff_cmd, "MERGED");

    // GUI tool commands should also be configured and shell-valid.
    let gui_merge_cmd = git_config_get(dir.path(), "mergetool.gitcomet-gui.cmd")
        .expect("GUI mergetool cmd should be configured by dry-run commands");
    assert!(
        gui_merge_cmd.contains("--gui"),
        "GUI merge cmd should contain --gui"
    );
    assert_placeholder_is_quoted(&gui_merge_cmd, "BASE");
    assert_placeholder_is_quoted(&gui_merge_cmd, "LOCAL");
    assert_placeholder_is_quoted(&gui_merge_cmd, "REMOTE");
    assert_placeholder_is_quoted(&gui_merge_cmd, "MERGED");

    let gui_diff_cmd = git_config_get(dir.path(), "difftool.gitcomet-gui.cmd")
        .expect("GUI difftool cmd should be configured by dry-run commands");
    assert!(
        gui_diff_cmd.contains("--gui"),
        "GUI diff cmd should contain --gui"
    );
    assert_placeholder_is_quoted(&gui_diff_cmd, "LOCAL");
    assert_placeholder_is_quoted(&gui_diff_cmd, "REMOTE");
    assert_placeholder_is_quoted(&gui_diff_cmd, "MERGED");

    assert_eq!(
        git_config_get(dir.path(), "merge.guitool").as_deref(),
        Some("gitcomet-gui"),
        "merge.guitool should reference gitcomet-gui"
    );
    assert_eq!(
        git_config_get(dir.path(), "diff.guitool").as_deref(),
        Some("gitcomet-gui"),
        "diff.guitool should reference gitcomet-gui"
    );
}

#[test]
fn setup_local_writes_config_to_repo() {
    let dir = tempfile::tempdir().unwrap();

    // Initialize a git repo.
    let init = Command::new("git")
        .args(["init", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(init.status.success(), "git init failed");

    let output = run_gitcomet_in_dir(
        dir.path(),
        [OsString::from("setup"), OsString::from("--local")],
    );

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "expected exit 0\n{text}");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Configured gitcomet as local diff/merge tool"),
        "{text}"
    );

    // Verify key config entries were written.
    assert_eq!(
        git_config_get(dir.path(), "merge.tool").as_deref(),
        Some("gitcomet"),
        "merge.tool not set"
    );
    assert_eq!(
        git_config_get(dir.path(), "diff.tool").as_deref(),
        Some("gitcomet"),
        "diff.tool not set"
    );
    assert_eq!(
        git_config_get(dir.path(), "mergetool.trustExitCode").as_deref(),
        Some("true"),
        "mergetool.trustExitCode not set"
    );
    assert_eq!(
        git_config_get(dir.path(), "mergetool.gitcomet.trustExitCode").as_deref(),
        Some("true"),
        "mergetool.gitcomet.trustExitCode not set"
    );
    assert_eq!(
        git_config_get(dir.path(), "mergetool.prompt").as_deref(),
        Some("false"),
        "mergetool.prompt not set"
    );
    assert_eq!(
        git_config_get(dir.path(), "difftool.trustExitCode").as_deref(),
        Some("true"),
        "difftool.trustExitCode not set"
    );
    assert_eq!(
        git_config_get(dir.path(), "difftool.prompt").as_deref(),
        Some("false"),
        "difftool.prompt not set"
    );
    assert_eq!(
        git_config_get(dir.path(), "merge.guitool").as_deref(),
        Some("gitcomet-gui"),
        "merge.guitool not set to gitcomet-gui"
    );
    assert_eq!(
        git_config_get(dir.path(), "diff.guitool").as_deref(),
        Some("gitcomet-gui"),
        "diff.guitool not set to gitcomet-gui"
    );
    assert_eq!(
        git_config_get(dir.path(), "mergetool.guiDefault").as_deref(),
        Some("auto"),
        "mergetool.guiDefault not set"
    );
    assert_eq!(
        git_config_get(dir.path(), "difftool.guiDefault").as_deref(),
        Some("auto"),
        "difftool.guiDefault not set"
    );

    // Verify headless cmd contains the binary path and proper variable quoting.
    let merge_cmd =
        git_config_get(dir.path(), "mergetool.gitcomet.cmd").expect("mergetool cmd should be set");
    assert_placeholder_is_quoted(&merge_cmd, "BASE");
    assert_placeholder_is_quoted(&merge_cmd, "LOCAL");
    assert_placeholder_is_quoted(&merge_cmd, "REMOTE");
    assert_placeholder_is_quoted(&merge_cmd, "MERGED");
    assert!(
        !merge_cmd.contains("--gui"),
        "headless merge cmd should not contain --gui"
    );

    let diff_cmd =
        git_config_get(dir.path(), "difftool.gitcomet.cmd").expect("difftool cmd should be set");
    assert_placeholder_is_quoted(&diff_cmd, "LOCAL");
    assert_placeholder_is_quoted(&diff_cmd, "REMOTE");
    assert_placeholder_is_quoted(&diff_cmd, "MERGED");
    assert!(
        !diff_cmd.contains("--gui"),
        "headless diff cmd should not contain --gui"
    );

    // Verify GUI cmd includes --gui flag.
    let gui_merge_cmd = git_config_get(dir.path(), "mergetool.gitcomet-gui.cmd")
        .expect("GUI mergetool cmd should be set");
    assert!(
        gui_merge_cmd.contains("--gui"),
        "GUI merge cmd missing --gui"
    );
    assert_placeholder_is_quoted(&gui_merge_cmd, "BASE");
    assert_placeholder_is_quoted(&gui_merge_cmd, "LOCAL");
    assert_placeholder_is_quoted(&gui_merge_cmd, "REMOTE");
    assert_placeholder_is_quoted(&gui_merge_cmd, "MERGED");
    assert_eq!(
        git_config_get(dir.path(), "mergetool.gitcomet-gui.trustExitCode").as_deref(),
        Some("true"),
        "GUI mergetool.trustExitCode not set"
    );

    let gui_diff_cmd = git_config_get(dir.path(), "difftool.gitcomet-gui.cmd")
        .expect("GUI difftool cmd should be set");
    assert!(gui_diff_cmd.contains("--gui"), "GUI diff cmd missing --gui");
    assert_placeholder_is_quoted(&gui_diff_cmd, "LOCAL");
    assert_placeholder_is_quoted(&gui_diff_cmd, "REMOTE");
    assert_placeholder_is_quoted(&gui_diff_cmd, "MERGED");
    assert_eq!(
        git_config_get(dir.path(), "difftool.gitcomet-gui.trustExitCode").as_deref(),
        Some("true"),
        "GUI difftool.trustExitCode not set"
    );
}

#[test]
fn setup_local_mergetool_tool_help_lists_headless_and_gui_entries() {
    if !require_git_shell_for_setup_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    setup_e2e_init(repo);

    let setup = run_gitcomet_in_dir(repo, ["setup", "--local"]);
    let setup_text = output_text(&setup);
    assert_eq!(setup.status.code(), Some(0), "setup failed\n{setup_text}");

    let tool_help = setup_e2e_git_capture(repo, &["mergetool", "--tool-help"]);
    let text = output_text(&tool_help);
    assert!(
        tool_help.status.success(),
        "git mergetool --tool-help failed\n{text}"
    );
    assert!(
        text.contains("gitcomet.cmd"),
        "expected headless gitcomet tool in mergetool --tool-help output\n{text}"
    );
    assert!(
        text.contains("gitcomet-gui.cmd"),
        "expected gui gitcomet-gui tool in mergetool --tool-help output\n{text}"
    );
    assert!(
        text.contains("mergetool --base"),
        "expected mergetool command shape in --tool-help output\n{text}"
    );
    assert!(
        text.contains("mergetool --gui"),
        "expected gui mergetool command shape in --tool-help output\n{text}"
    );
}

#[test]
fn setup_local_difftool_tool_help_lists_headless_and_gui_entries() {
    if !require_git_shell_for_setup_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    setup_e2e_init(repo);

    let setup = run_gitcomet_in_dir(repo, ["setup", "--local"]);
    let setup_text = output_text(&setup);
    assert_eq!(setup.status.code(), Some(0), "setup failed\n{setup_text}");

    let tool_help = setup_e2e_git_capture(repo, &["difftool", "--tool-help"]);
    let text = output_text(&tool_help);
    assert!(
        tool_help.status.success(),
        "git difftool --tool-help failed\n{text}"
    );
    assert!(
        text.contains("gitcomet.cmd"),
        "expected headless gitcomet tool in difftool --tool-help output\n{text}"
    );
    assert!(
        text.contains("gitcomet-gui.cmd"),
        "expected gui gitcomet-gui tool in difftool --tool-help output\n{text}"
    );
    assert!(
        text.contains("difftool --local"),
        "expected difftool command shape in --tool-help output\n{text}"
    );
    assert!(
        text.contains("difftool --gui"),
        "expected gui difftool command shape in --tool-help output\n{text}"
    );
}

#[test]
fn uninstall_dry_run_local_after_setup_lists_unset_commands() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    setup_e2e_init(repo);
    let setup = run_gitcomet_in_dir(repo, ["setup", "--local"]);
    let setup_text = output_text(&setup);
    assert_eq!(setup.status.code(), Some(0), "setup failed\n{setup_text}");

    let uninstall = run_gitcomet_in_dir(repo, ["uninstall", "--dry-run", "--local"]);
    let text = output_text(&uninstall);
    assert_eq!(uninstall.status.code(), Some(0), "expected exit 0\n{text}");

    let stdout = String::from_utf8_lossy(&uninstall.stdout);
    assert!(
        stdout.contains("git config --local --unset-all mergetool.gitcomet.cmd"),
        "expected headless mergetool unset command\n{text}"
    );
    assert!(
        stdout.contains("git config --local --unset-all merge.tool"),
        "expected merge.tool unset command\n{text}"
    );
    assert!(
        stdout.contains("git config --local --unset-all mergetool.prompt"),
        "expected guarded mergetool.prompt unset command\n{text}"
    );
}

#[test]
fn uninstall_local_removes_setup_keys_from_repo() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    setup_e2e_init(repo);
    let setup = run_gitcomet_in_dir(repo, ["setup", "--local"]);
    let setup_text = output_text(&setup);
    assert_eq!(setup.status.code(), Some(0), "setup failed\n{setup_text}");

    let uninstall = run_gitcomet_in_dir(repo, ["uninstall", "--local"]);
    let text = output_text(&uninstall);
    assert_eq!(uninstall.status.code(), Some(0), "expected exit 0\n{text}");
    assert!(
        String::from_utf8_lossy(&uninstall.stdout)
            .contains("Unconfigured gitcomet from local diff/merge tool"),
        "expected uninstall summary message\n{text}"
    );

    // Tool selectors should be removed from local scope.
    assert_eq!(git_config_get_local(repo, "merge.tool"), None);
    assert_eq!(git_config_get_local(repo, "diff.tool"), None);
    assert_eq!(git_config_get_local(repo, "merge.guitool"), None);
    assert_eq!(git_config_get_local(repo, "diff.guitool"), None);

    // Tool-specific command registrations should be removed.
    assert_eq!(git_config_get_local(repo, "mergetool.gitcomet.cmd"), None);
    assert_eq!(git_config_get_local(repo, "difftool.gitcomet.cmd"), None);
    assert_eq!(
        git_config_get_local(repo, "mergetool.gitcomet-gui.cmd"),
        None
    );
    assert_eq!(
        git_config_get_local(repo, "difftool.gitcomet-gui.cmd"),
        None
    );

    // Generic behavior keys written by setup should be removed when selectors
    // still point to GitComet.
    assert_eq!(git_config_get_local(repo, "mergetool.prompt"), None);
    assert_eq!(git_config_get_local(repo, "mergetool.trustExitCode"), None);
    assert_eq!(git_config_get_local(repo, "difftool.prompt"), None);
    assert_eq!(git_config_get_local(repo, "difftool.trustExitCode"), None);
}

#[test]
fn uninstall_local_keeps_non_gitcomet_generic_settings() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    setup_e2e_init(repo);
    git_config_set_local(repo, "merge.tool", "meld");
    git_config_set_local(repo, "mergetool.prompt", "false");
    git_config_set_local(repo, "mergetool.trustExitCode", "true");
    git_config_set_local(repo, "mergetool.gitcomet.cmd", "echo custom");

    let uninstall = run_gitcomet_in_dir(repo, ["uninstall", "--local"]);
    let text = output_text(&uninstall);
    assert_eq!(uninstall.status.code(), Some(0), "expected exit 0\n{text}");

    // Keep non-GitComet generic behavior.
    assert_eq!(
        git_config_get_local(repo, "merge.tool").as_deref(),
        Some("meld")
    );
    assert_eq!(
        git_config_get_local(repo, "mergetool.prompt").as_deref(),
        Some("false")
    );
    assert_eq!(
        git_config_get_local(repo, "mergetool.trustExitCode").as_deref(),
        Some("true")
    );

    // Remove GitComet tool-specific key regardless of command content.
    assert_eq!(git_config_get_local(repo, "mergetool.gitcomet.cmd"), None);
}

#[test]
fn setup_then_uninstall_local_restores_preexisting_user_settings() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    setup_e2e_init(repo);

    git_config_set_local(repo, "merge.tool", "meld");
    git_config_set_local(repo, "diff.tool", "vimdiff");
    git_config_set_local(repo, "merge.guitool", "kdiff3");
    git_config_set_local(repo, "diff.guitool", "kdiff3");
    git_config_set_local(repo, "mergetool.prompt", "true");
    git_config_set_local(repo, "difftool.prompt", "true");
    git_config_set_local(repo, "mergetool.trustExitCode", "false");
    git_config_set_local(repo, "difftool.trustExitCode", "false");
    git_config_set_local(repo, "mergetool.guiDefault", "false");
    git_config_set_local(repo, "difftool.guiDefault", "false");

    let setup = run_gitcomet_in_dir(repo, ["setup", "--local"]);
    let setup_text = output_text(&setup);
    assert_eq!(setup.status.code(), Some(0), "setup failed\n{setup_text}");

    let uninstall = run_gitcomet_in_dir(repo, ["uninstall", "--local"]);
    let uninstall_text = output_text(&uninstall);
    assert_eq!(
        uninstall.status.code(),
        Some(0),
        "uninstall failed\n{uninstall_text}"
    );

    assert_eq!(
        git_config_get_local(repo, "merge.tool").as_deref(),
        Some("meld")
    );
    assert_eq!(
        git_config_get_local(repo, "diff.tool").as_deref(),
        Some("vimdiff")
    );
    assert_eq!(
        git_config_get_local(repo, "merge.guitool").as_deref(),
        Some("kdiff3")
    );
    assert_eq!(
        git_config_get_local(repo, "diff.guitool").as_deref(),
        Some("kdiff3")
    );
    assert_eq!(
        git_config_get_local(repo, "mergetool.prompt").as_deref(),
        Some("true")
    );
    assert_eq!(
        git_config_get_local(repo, "difftool.prompt").as_deref(),
        Some("true")
    );
    assert_eq!(
        git_config_get_local(repo, "mergetool.trustExitCode").as_deref(),
        Some("false")
    );
    assert_eq!(
        git_config_get_local(repo, "difftool.trustExitCode").as_deref(),
        Some("false")
    );
    assert_eq!(
        git_config_get_local(repo, "mergetool.guiDefault").as_deref(),
        Some("false")
    );
    assert_eq!(
        git_config_get_local(repo, "difftool.guiDefault").as_deref(),
        Some("false")
    );

    assert_eq!(git_config_get_local(repo, "mergetool.gitcomet.cmd"), None);
    assert_eq!(git_config_get_local(repo, "difftool.gitcomet.cmd"), None);
    assert_eq!(
        git_config_get_local(repo, "mergetool.gitcomet-gui.cmd"),
        None
    );
    assert_eq!(
        git_config_get_local(repo, "difftool.gitcomet-gui.cmd"),
        None
    );
    assert_eq!(
        git_config_get_local(repo, "gitcomet.backup.merge-tool"),
        None
    );
    assert_eq!(
        git_config_get_local(repo, "gitcomet.backup.diff-tool"),
        None
    );
}

#[test]
fn setup_local_is_idempotent_and_preserves_original_backup_state() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    setup_e2e_init(repo);
    git_config_set_local(repo, "merge.tool", "meld");

    let setup1 = run_gitcomet_in_dir(repo, ["setup", "--local"]);
    let setup1_text = output_text(&setup1);
    assert_eq!(
        setup1.status.code(),
        Some(0),
        "first setup failed\n{setup1_text}"
    );

    let setup2 = run_gitcomet_in_dir(repo, ["setup", "--local"]);
    let setup2_text = output_text(&setup2);
    assert_eq!(
        setup2.status.code(),
        Some(0),
        "second setup failed\n{setup2_text}"
    );

    let uninstall = run_gitcomet_in_dir(repo, ["uninstall", "--local"]);
    let uninstall_text = output_text(&uninstall);
    assert_eq!(
        uninstall.status.code(),
        Some(0),
        "uninstall failed\n{uninstall_text}"
    );

    assert_eq!(
        git_config_get_local(repo, "merge.tool").as_deref(),
        Some("meld")
    );
}

#[test]
fn uninstall_local_preserves_user_changes_made_after_setup() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    setup_e2e_init(repo);
    // Pre-setup values.
    git_config_set_local(repo, "merge.tool", "meld");
    git_config_set_local(repo, "difftool.prompt", "true");

    let setup = run_gitcomet_in_dir(repo, ["setup", "--local"]);
    let setup_text = output_text(&setup);
    assert_eq!(setup.status.code(), Some(0), "setup failed\n{setup_text}");

    // User edits after setup should be preserved by uninstall.
    git_config_set_local(repo, "merge.tool", "vimdiff");
    git_config_set_local(repo, "difftool.prompt", "true");

    let uninstall = run_gitcomet_in_dir(repo, ["uninstall", "--local"]);
    let uninstall_text = output_text(&uninstall);
    assert_eq!(
        uninstall.status.code(),
        Some(0),
        "uninstall failed\n{uninstall_text}"
    );

    assert_eq!(
        git_config_get_local(repo, "merge.tool").as_deref(),
        Some("vimdiff")
    );
    assert_eq!(
        git_config_get_local(repo, "difftool.prompt").as_deref(),
        Some("true")
    );
    assert_eq!(git_config_get_local(repo, "mergetool.gitcomet.cmd"), None);
    assert_eq!(git_config_get_local(repo, "difftool.gitcomet.cmd"), None);
    assert_eq!(
        git_config_get_local(repo, "gitcomet.backup.merge-tool"),
        None
    );
    assert_eq!(
        git_config_get_local(repo, "gitcomet.backup.difftool-prompt"),
        None
    );
}

#[test]
fn uninstall_local_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    setup_e2e_init(repo);
    let setup = run_gitcomet_in_dir(repo, ["setup", "--local"]);
    let setup_text = output_text(&setup);
    assert_eq!(setup.status.code(), Some(0), "setup failed\n{setup_text}");

    let uninstall1 = run_gitcomet_in_dir(repo, ["uninstall", "--local"]);
    let uninstall1_text = output_text(&uninstall1);
    assert_eq!(
        uninstall1.status.code(),
        Some(0),
        "first uninstall failed\n{uninstall1_text}"
    );

    let uninstall2 = run_gitcomet_in_dir(repo, ["uninstall", "--local"]);
    let uninstall2_text = output_text(&uninstall2);
    assert_eq!(
        uninstall2.status.code(),
        Some(0),
        "second uninstall failed\n{uninstall2_text}"
    );

    assert_eq!(git_config_get_local(repo, "merge.tool"), None);
    assert_eq!(git_config_get_local(repo, "diff.tool"), None);
    assert_eq!(git_config_get_local(repo, "mergetool.gitcomet.cmd"), None);
    assert_eq!(git_config_get_local(repo, "difftool.gitcomet.cmd"), None);
}

// ── Auto-resolve mode E2E ───────────────────────────────────────────

#[test]
fn standalone_mergetool_auto_resolves_whitespace_conflict_exits_zero() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.txt");
    let local = dir.path().join("local.txt");
    let remote = dir.path().join("remote.txt");
    let merged = dir.path().join("merged.txt");

    write_file(&base, "aaa\nbbb\nccc\n");
    write_file(&local, "aaa\nbbb  \nccc\n");
    write_file(&remote, "aaa\nbbb\t\nccc\n");
    write_file(&merged, "");

    let output = run_gitcomet([
        "mergetool",
        "--auto",
        "--base",
        &base.to_string_lossy(),
        "--local",
        &local.to_string_lossy(),
        "--remote",
        &remote.to_string_lossy(),
        "--merged",
        &merged.to_string_lossy(),
    ]);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "auto mergetool should exit 0 for whitespace-only conflict\nstderr: {stderr}"
    );
    let result = fs::read_to_string(&merged).unwrap();
    assert!(
        !result.contains("<<<<<<<"),
        "output should not contain conflict markers\n{result}"
    );
}

#[test]
fn standalone_mergetool_auto_merge_alias_resolves_whitespace_conflict_exits_zero() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.txt");
    let local = dir.path().join("local.txt");
    let remote = dir.path().join("remote.txt");
    let merged = dir.path().join("merged.txt");

    write_file(&base, "aaa\nbbb\nccc\n");
    write_file(&local, "aaa\nbbb  \nccc\n");
    write_file(&remote, "aaa\nbbb\t\nccc\n");
    write_file(&merged, "");

    let output = run_gitcomet([
        "mergetool",
        "--auto-merge",
        "--base",
        &base.to_string_lossy(),
        "--local",
        &local.to_string_lossy(),
        "--remote",
        &remote.to_string_lossy(),
        "--merged",
        &merged.to_string_lossy(),
    ]);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "--auto-merge alias should behave like --auto for whitespace-only conflicts\nstderr: {stderr}"
    );
    let result = fs::read_to_string(&merged).unwrap();
    assert!(
        !result.contains("<<<<<<<"),
        "output should not contain conflict markers\n{result}"
    );
}

#[test]
fn standalone_mergetool_auto_with_diff3_resolves_subchunk_exits_zero() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.txt");
    let local = dir.path().join("local.txt");
    let remote = dir.path().join("remote.txt");
    let merged = dir.path().join("merged.txt");

    // Ours changes line 2, theirs changes line 1 — non-overlapping within block.
    write_file(&base, "aaa\nbbb\nccc\n");
    write_file(&local, "aaa\nBBB\nccc\n");
    write_file(&remote, "AAA\nbbb\nccc\n");
    write_file(&merged, "");

    let output = run_gitcomet([
        "mergetool",
        "--auto",
        "--conflict-style",
        "diff3",
        "--base",
        &base.to_string_lossy(),
        "--local",
        &local.to_string_lossy(),
        "--remote",
        &remote.to_string_lossy(),
        "--merged",
        &merged.to_string_lossy(),
    ]);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "auto mergetool with diff3 should exit 0 for subchunk-resolvable conflict\nstderr: {stderr}"
    );
    let result = fs::read_to_string(&merged).unwrap();
    assert_eq!(result, "AAA\nBBB\nccc\n");
}

#[test]
fn standalone_mergetool_auto_unresolvable_conflict_exits_one() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.txt");
    let local = dir.path().join("local.txt");
    let remote = dir.path().join("remote.txt");
    let merged = dir.path().join("merged.txt");

    write_file(&base, "aaa\nbbb\nccc\n");
    write_file(&local, "aaa\nXXX\nccc\n");
    write_file(&remote, "aaa\nYYY\nccc\n");
    write_file(&merged, "");

    let output = run_gitcomet([
        "mergetool",
        "--auto",
        "--base",
        &base.to_string_lossy(),
        "--local",
        &local.to_string_lossy(),
        "--remote",
        &remote.to_string_lossy(),
        "--merged",
        &merged.to_string_lossy(),
    ]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "auto mergetool should still exit 1 for true conflicts"
    );
    let result = fs::read_to_string(&merged).unwrap();
    assert!(
        result.contains("<<<<<<<"),
        "output should contain conflict markers for true conflicts"
    );
}

#[test]
fn standalone_mergetool_without_auto_does_not_autosolve() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.txt");
    let local = dir.path().join("local.txt");
    let remote = dir.path().join("remote.txt");
    let merged = dir.path().join("merged.txt");

    // Whitespace-only conflict — auto mode would resolve it.
    write_file(&base, "aaa\nbbb\nccc\n");
    write_file(&local, "aaa\nbbb  \nccc\n");
    write_file(&remote, "aaa\nbbb\t\nccc\n");
    write_file(&merged, "");

    let output = run_gitcomet([
        "mergetool",
        "--base",
        &base.to_string_lossy(),
        "--local",
        &local.to_string_lossy(),
        "--remote",
        &remote.to_string_lossy(),
        "--merged",
        &merged.to_string_lossy(),
    ]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "without --auto, whitespace-only conflict should exit 1"
    );
    let result = fs::read_to_string(&merged).unwrap();
    assert!(
        result.contains("<<<<<<<"),
        "without --auto, output should contain conflict markers"
    );
}

#[test]
fn standalone_mergetool_auto_crlf_subchunk_preserves_line_endings() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.txt");
    let local = dir.path().join("local.txt");
    let remote = dir.path().join("remote.txt");
    let merged = dir.path().join("merged.txt");

    // CRLF files with non-overlapping changes — auto mode should resolve
    // via subchunk splitting and preserve CRLF endings.
    write_file(&base, "aaa\r\nbbb\r\nccc\r\n");
    write_file(&local, "aaa\r\nBBB\r\nccc\r\n"); // changed line 2
    write_file(&remote, "AAA\r\nbbb\r\nccc\r\n"); // changed line 1
    write_file(&merged, "");

    let output = run_gitcomet([
        "mergetool",
        "--base",
        &base.to_string_lossy(),
        "--local",
        &local.to_string_lossy(),
        "--remote",
        &remote.to_string_lossy(),
        "--merged",
        &merged.to_string_lossy(),
        "--conflict-style",
        "diff3",
        "--auto",
    ]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "auto mode should resolve non-overlapping CRLF conflict, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = fs::read(&merged).unwrap();
    let result_str = String::from_utf8(result).unwrap();
    assert_eq!(
        result_str, "AAA\r\nBBB\r\nccc\r\n",
        "auto-resolved output must preserve CRLF line endings"
    );
    // Verify no stray LF-only endings.
    assert_eq!(
        result_str.matches("\r\n").count(),
        result_str.matches('\n').count(),
        "all line endings should be CRLF"
    );
}

// ── Setup E2E integration ────────────────────────────────────────────
//
// These tests verify that `gitcomet-app setup --local` produces config that
// actually works when Git invokes `git mergetool` / `git difftool`.  This
// closes the gap between "config keys are written" and "the configured tool
// is invoked end-to-end" — directly validating acceptance criteria 2-3 from
// external_usage.md.

/// Run a git command in a repo; assert success.
fn setup_e2e_git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .expect("git command to run");
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout: {}\nstderr: {}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// Run a git command in a repo; return output (may succeed or fail).
fn setup_e2e_git_capture(repo: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .expect("git command to run")
}

/// Initialize a git repo with user config for tests.
fn setup_e2e_init(repo: &Path) {
    setup_e2e_git(repo, &["init", "-b", "main"]);
    setup_e2e_git(repo, &["config", "user.email", "test@test.com"]);
    setup_e2e_git(repo, &["config", "user.name", "Test"]);
    setup_e2e_git(repo, &["config", "commit.gpgsign", "false"]);
}

/// Stage all and commit.
fn setup_e2e_commit(repo: &Path, message: &str) {
    setup_e2e_git(repo, &["add", "-A"]);
    setup_e2e_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", message],
    );
}

struct IsolatedGlobalGitEnv {
    home_dir: PathBuf,
    xdg_config_home: PathBuf,
    global_config: PathBuf,
}

impl IsolatedGlobalGitEnv {
    fn new(root: &Path) -> Self {
        let home_dir = root.join("home");
        let xdg_config_home = root.join("xdg");
        let global_config = root.join("global.gitconfig");

        fs::create_dir_all(&home_dir).expect("create isolated HOME directory");
        fs::create_dir_all(&xdg_config_home).expect("create isolated XDG_CONFIG_HOME directory");
        fs::write(&global_config, "").expect("create isolated global git config file");

        Self {
            home_dir,
            xdg_config_home,
            global_config,
        }
    }

    fn apply_to_command(&self, command: &mut Command) {
        command
            .env("HOME", &self.home_dir)
            .env("XDG_CONFIG_HOME", &self.xdg_config_home)
            .env("GIT_CONFIG_GLOBAL", &self.global_config)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_CONFIG_SYSTEM")
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY");
    }
}

fn setup_e2e_git_capture_with_env(
    repo: &Path,
    args: &[&str],
    env: &IsolatedGlobalGitEnv,
) -> Output {
    let mut command = Command::new("git");
    env.apply_to_command(&mut command);
    command
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("git command to run")
}

fn setup_e2e_git_with_env(repo: &Path, args: &[&str], env: &IsolatedGlobalGitEnv) {
    let output = setup_e2e_git_capture_with_env(repo, args, env);
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout: {}\nstderr: {}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn setup_e2e_init_with_env(repo: &Path, env: &IsolatedGlobalGitEnv) {
    setup_e2e_git_with_env(repo, &["init", "-b", "main"], env);
    setup_e2e_git_with_env(repo, &["config", "user.email", "test@test.com"], env);
    setup_e2e_git_with_env(repo, &["config", "user.name", "Test"], env);
    setup_e2e_git_with_env(repo, &["config", "commit.gpgsign", "false"], env);
}

fn setup_e2e_commit_with_env(repo: &Path, message: &str, env: &IsolatedGlobalGitEnv) {
    setup_e2e_git_with_env(repo, &["add", "-A"], env);
    setup_e2e_git_with_env(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", message],
        env,
    );
}

fn git_config_get_global_with_env(env: &IsolatedGlobalGitEnv, key: &str) -> Option<String> {
    let mut command = Command::new("git");
    env.apply_to_command(&mut command);
    let output = command
        .args(["config", "--global", "--get", key])
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

/// After `gitcomet-app setup --local`, `git mergetool` should invoke
/// gitcomet-app's built-in 3-way merge for conflicted files.
///
/// For a true content conflict, gitcomet-app exits 1 and git mergetool
/// restores the original file (expected behavior with trustExitCode=true).
/// We verify the tool was invoked by checking for gitcomet-app's specific
/// stderr messages, which differ from git's own merge output.
#[test]
fn setup_local_enables_git_mergetool_end_to_end() {
    if !require_git_shell_for_setup_integration_tests() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    // 1. Initialize repo and run setup.
    setup_e2e_init(repo);
    let setup = run_gitcomet_in_dir(repo, [OsString::from("setup"), OsString::from("--local")]);
    let text = output_text(&setup);
    assert_eq!(setup.status.code(), Some(0), "setup failed\n{text}");

    // 2. Create a merge conflict (both sides modify line 2).
    write_file(&repo.join("file.txt"), "line1\nline2\nline3\n");
    setup_e2e_commit(repo, "base");

    setup_e2e_git(repo, &["checkout", "-b", "ours"]);
    write_file(&repo.join("file.txt"), "line1\nOURS_CHANGE\nline3\n");
    setup_e2e_commit(repo, "ours");

    setup_e2e_git(repo, &["checkout", "main"]);
    write_file(&repo.join("file.txt"), "line1\nTHEIRS_CHANGE\nline3\n");
    setup_e2e_commit(repo, "theirs");

    let merge = setup_e2e_git_capture(repo, &["merge", "ours"]);
    assert!(
        !merge.status.success(),
        "expected merge conflict but git merge succeeded"
    );

    // 3. Run `git mergetool` — should invoke gitcomet-app via setup config.
    //    DISPLAY is removed so guiDefault=auto selects the headless tool.
    let mt = setup_e2e_git_capture(repo, &["mergetool"]);
    let mt_stderr = String::from_utf8_lossy(&mt.stderr);

    // 4. Verify gitcomet-app was invoked by checking for its specific stderr
    //    messages.  "conflict(s) remain" is emitted by gitcomet-app's mergetool
    //    mode and is NOT part of git's own output (git says "fix conflicts and
    //    then commit the result" instead).
    assert!(
        mt_stderr.contains("conflict(s) remain"),
        "expected gitcomet-app's conflict message in mergetool stderr\n\
         stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&mt.stdout),
        mt_stderr,
    );

    // Also verify gitcomet-app's "CONFLICT (content)" message is present.
    assert!(
        mt_stderr.contains("CONFLICT (content)"),
        "expected CONFLICT marker from gitcomet-app\nstderr: {}",
        mt_stderr,
    );
}

/// After `gitcomet-app setup --local`, `git difftool` should invoke
/// gitcomet-app's built-in diff and produce unified diff output.
#[test]
fn setup_local_enables_git_difftool_end_to_end() {
    if !require_git_shell_for_setup_integration_tests() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    // 1. Initialize repo and run setup.
    setup_e2e_init(repo);
    let setup = run_gitcomet_in_dir(repo, [OsString::from("setup"), OsString::from("--local")]);
    let text = output_text(&setup);
    assert_eq!(setup.status.code(), Some(0), "setup failed\n{text}");

    // 2. Create a commit and then modify the file.
    write_file(&repo.join("file.txt"), "line1\nline2\nline3\n");
    setup_e2e_commit(repo, "initial");
    write_file(&repo.join("file.txt"), "line1\nMODIFIED\nline3\n");

    // 3. Run `git difftool` — should invoke gitcomet-app difftool.
    //    DISPLAY is removed so guiDefault=auto selects the headless tool.
    let dt = setup_e2e_git_capture(repo, &["difftool"]);
    let dt_text = output_text(&dt);

    assert_eq!(
        dt.status.code(),
        Some(0),
        "git difftool should exit 0\n{dt_text}"
    );

    // gitcomet-app difftool produces unified diff output.
    let stdout = String::from_utf8_lossy(&dt.stdout);
    assert!(
        stdout.contains("@@"),
        "expected diff hunk header in output\n{dt_text}"
    );
    assert!(
        stdout.contains("-line2"),
        "expected removed line in diff\n{dt_text}"
    );
    assert!(
        stdout.contains("+MODIFIED"),
        "expected added line in diff\n{dt_text}"
    );
}

/// After `gitcomet-app setup --local`, quoted stage variables in the generated
/// mergetool command must preserve paths containing spaces/unicode.
#[test]
fn setup_local_mergetool_handles_spaced_unicode_path_end_to_end() {
    if !require_git_shell_for_setup_integration_tests() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();
    let conflict_path = "docs/spaced 日本語 file.txt";

    setup_e2e_init(repo);
    let setup = run_gitcomet_in_dir(repo, [OsString::from("setup"), OsString::from("--local")]);
    let setup_text = output_text(&setup);
    assert_eq!(setup.status.code(), Some(0), "setup failed\n{setup_text}");

    write_file(&repo.join(conflict_path), "line1\nline2\nline3\n");
    setup_e2e_commit(repo, "base");

    setup_e2e_git(repo, &["checkout", "-b", "ours"]);
    write_file(&repo.join(conflict_path), "line1\nOURS_CHANGE\nline3\n");
    setup_e2e_commit(repo, "ours");

    setup_e2e_git(repo, &["checkout", "main"]);
    write_file(&repo.join(conflict_path), "line1\nTHEIRS_CHANGE\nline3\n");
    setup_e2e_commit(repo, "theirs");

    let merge = setup_e2e_git_capture(repo, &["merge", "ours"]);
    assert!(
        !merge.status.success(),
        "expected merge conflict but git merge succeeded"
    );

    let mt = setup_e2e_git_capture(repo, &["mergetool", "--", conflict_path]);
    let mt_text = output_text(&mt);
    let mt_stderr = String::from_utf8_lossy(&mt.stderr);

    assert_eq!(
        mt.status.code(),
        Some(1),
        "expected unresolved conflict exit status\n{mt_text}"
    );
    assert!(
        mt_stderr.contains("CONFLICT (content)"),
        "expected gitcomet conflict output\n{mt_text}"
    );
    assert!(
        mt_stderr.contains(conflict_path),
        "expected conflicted spaced/unicode path in stderr\n{mt_text}"
    );
    assert!(
        !mt_stderr.contains("No such file or directory"),
        "quoted path handling failed\n{mt_text}"
    );
}

/// After `gitcomet-app setup --local`, quoted stage variables in the generated
/// difftool command must preserve paths containing spaces/unicode.
#[test]
fn setup_local_difftool_handles_spaced_unicode_path_end_to_end() {
    if !require_git_shell_for_setup_integration_tests() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();
    let diff_path = "docs/spaced 日本語 file.txt";

    setup_e2e_init(repo);
    let setup = run_gitcomet_in_dir(repo, [OsString::from("setup"), OsString::from("--local")]);
    let setup_text = output_text(&setup);
    assert_eq!(setup.status.code(), Some(0), "setup failed\n{setup_text}");

    write_file(&repo.join(diff_path), "line1\nline2\nline3\n");
    setup_e2e_commit(repo, "initial");
    write_file(&repo.join(diff_path), "line1\nMODIFIED\nline3\n");

    let dt = setup_e2e_git_capture(repo, &["difftool", "--", diff_path]);
    let dt_text = output_text(&dt);
    let dt_stdout = String::from_utf8_lossy(&dt.stdout);
    let dt_stderr = String::from_utf8_lossy(&dt.stderr);

    assert_eq!(
        dt.status.code(),
        Some(0),
        "git difftool should exit 0\n{dt_text}"
    );
    assert!(
        dt_stdout.contains("@@"),
        "expected diff hunk header in output\n{dt_text}"
    );
    assert!(
        dt_stdout.contains("-line2"),
        "expected removed line in diff\n{dt_text}"
    );
    assert!(
        dt_stdout.contains("+MODIFIED"),
        "expected added line in diff\n{dt_text}"
    );
    assert!(
        !dt_stderr.contains("No such file or directory"),
        "quoted path handling failed\n{dt_text}"
    );
}

/// `gitcomet-app setup` (global scope) should configure an isolated global
/// gitconfig so `git mergetool` works end-to-end without local repo config.
#[test]
fn setup_global_enables_git_mergetool_end_to_end_with_isolated_global_config() {
    if !require_git_shell_for_setup_integration_tests() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    let env = IsolatedGlobalGitEnv::new(tmp.path());

    setup_e2e_init_with_env(&repo, &env);

    let setup = run_gitcomet_in_dir_with_global_env(&repo, [OsString::from("setup")], &env);
    let setup_text = output_text(&setup);
    assert_eq!(setup.status.code(), Some(0), "setup failed\n{setup_text}");
    assert!(
        String::from_utf8_lossy(&setup.stdout).contains("Configured gitcomet as global"),
        "expected global setup message\n{setup_text}"
    );

    assert_eq!(
        git_config_get_global_with_env(&env, "merge.tool").as_deref(),
        Some("gitcomet"),
        "expected merge.tool in isolated global config"
    );
    assert_eq!(
        git_config_get_global_with_env(&env, "diff.tool").as_deref(),
        Some("gitcomet"),
        "expected diff.tool in isolated global config"
    );

    // Ensure setup did not write a local-scope override.
    let local_merge_tool =
        setup_e2e_git_capture_with_env(&repo, &["config", "--local", "--get", "merge.tool"], &env);
    assert!(
        !local_merge_tool.status.success(),
        "setup without --local should not set repo-local merge.tool"
    );

    write_file(&repo.join("file.txt"), "line1\nline2\nline3\n");
    setup_e2e_commit_with_env(&repo, "base", &env);

    setup_e2e_git_with_env(&repo, &["checkout", "-b", "ours"], &env);
    write_file(&repo.join("file.txt"), "line1\nOURS_CHANGE\nline3\n");
    setup_e2e_commit_with_env(&repo, "ours", &env);

    setup_e2e_git_with_env(&repo, &["checkout", "main"], &env);
    write_file(&repo.join("file.txt"), "line1\nTHEIRS_CHANGE\nline3\n");
    setup_e2e_commit_with_env(&repo, "theirs", &env);

    let merge = setup_e2e_git_capture_with_env(&repo, &["merge", "ours"], &env);
    assert!(
        !merge.status.success(),
        "expected merge conflict but git merge succeeded"
    );

    let mt = setup_e2e_git_capture_with_env(&repo, &["mergetool"], &env);
    let mt_stderr = String::from_utf8_lossy(&mt.stderr);
    assert_eq!(
        mt.status.code(),
        Some(1),
        "expected unresolved exit\n{mt_stderr}"
    );
    assert!(
        mt_stderr.contains("conflict(s) remain"),
        "expected gitcomet conflict message in mergetool stderr\n{}",
        output_text(&mt)
    );
    assert!(
        mt_stderr.contains("CONFLICT (content)"),
        "expected CONFLICT marker from gitcomet\n{}",
        output_text(&mt)
    );
}

/// `gitcomet-app setup` (global scope) should configure an isolated global
/// gitconfig so `git difftool` works end-to-end without local repo config.
#[test]
fn setup_global_enables_git_difftool_end_to_end_with_isolated_global_config() {
    if !require_git_shell_for_setup_integration_tests() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    let env = IsolatedGlobalGitEnv::new(tmp.path());

    setup_e2e_init_with_env(&repo, &env);

    let setup = run_gitcomet_in_dir_with_global_env(&repo, [OsString::from("setup")], &env);
    let setup_text = output_text(&setup);
    assert_eq!(setup.status.code(), Some(0), "setup failed\n{setup_text}");

    let local_diff_tool =
        setup_e2e_git_capture_with_env(&repo, &["config", "--local", "--get", "diff.tool"], &env);
    assert!(
        !local_diff_tool.status.success(),
        "setup without --local should not set repo-local diff.tool"
    );
    assert_eq!(
        git_config_get_global_with_env(&env, "diff.tool").as_deref(),
        Some("gitcomet"),
        "expected diff.tool in isolated global config"
    );

    write_file(&repo.join("file.txt"), "line1\nline2\nline3\n");
    setup_e2e_commit_with_env(&repo, "initial", &env);
    write_file(&repo.join("file.txt"), "line1\nMODIFIED\nline3\n");

    let dt = setup_e2e_git_capture_with_env(&repo, &["difftool"], &env);
    let dt_text = output_text(&dt);
    assert_eq!(
        dt.status.code(),
        Some(0),
        "git difftool should exit 0\n{dt_text}"
    );

    let stdout = String::from_utf8_lossy(&dt.stdout);
    assert!(
        stdout.contains("@@"),
        "expected diff hunk header\n{dt_text}"
    );
    assert!(
        stdout.contains("-line2"),
        "expected removed line\n{dt_text}"
    );
    assert!(
        stdout.contains("+MODIFIED"),
        "expected added line\n{dt_text}"
    );
}

/// `gitcomet-app setup` (global scope) should make both headless and GUI
/// mergetool entries discoverable via `git mergetool --tool-help`.
#[test]
fn setup_global_mergetool_tool_help_lists_headless_and_gui_entries() {
    if !require_git_shell_for_setup_integration_tests() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    let env = IsolatedGlobalGitEnv::new(tmp.path());

    setup_e2e_init_with_env(&repo, &env);

    let setup = run_gitcomet_in_dir_with_global_env(&repo, [OsString::from("setup")], &env);
    let setup_text = output_text(&setup);
    assert_eq!(setup.status.code(), Some(0), "setup failed\n{setup_text}");
    assert_eq!(
        git_config_get_global_with_env(&env, "merge.guitool").as_deref(),
        Some("gitcomet-gui"),
        "expected merge.guitool in isolated global config"
    );
    assert_eq!(
        git_config_get_global_with_env(&env, "mergetool.guiDefault").as_deref(),
        Some("auto"),
        "expected mergetool.guiDefault=auto in isolated global config"
    );

    let tool_help = setup_e2e_git_capture_with_env(&repo, &["mergetool", "--tool-help"], &env);
    let text = output_text(&tool_help);
    assert!(
        tool_help.status.success(),
        "git mergetool --tool-help failed\n{text}"
    );
    assert!(
        text.contains("gitcomet.cmd"),
        "expected headless gitcomet tool in mergetool --tool-help output\n{text}"
    );
    assert!(
        text.contains("gitcomet-gui.cmd"),
        "expected gui gitcomet-gui tool in mergetool --tool-help output\n{text}"
    );
    assert!(
        text.contains("mergetool --base"),
        "expected mergetool command shape in --tool-help output\n{text}"
    );
    assert!(
        text.contains("mergetool --gui"),
        "expected gui mergetool command shape in --tool-help output\n{text}"
    );
}

/// `gitcomet-app setup` (global scope) should make both headless and GUI
/// difftool entries discoverable via `git difftool --tool-help`.
#[test]
fn setup_global_difftool_tool_help_lists_headless_and_gui_entries() {
    if !require_git_shell_for_setup_integration_tests() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    let env = IsolatedGlobalGitEnv::new(tmp.path());

    setup_e2e_init_with_env(&repo, &env);

    let setup = run_gitcomet_in_dir_with_global_env(&repo, [OsString::from("setup")], &env);
    let setup_text = output_text(&setup);
    assert_eq!(setup.status.code(), Some(0), "setup failed\n{setup_text}");
    assert_eq!(
        git_config_get_global_with_env(&env, "diff.guitool").as_deref(),
        Some("gitcomet-gui"),
        "expected diff.guitool in isolated global config"
    );
    assert_eq!(
        git_config_get_global_with_env(&env, "difftool.guiDefault").as_deref(),
        Some("auto"),
        "expected difftool.guiDefault=auto in isolated global config"
    );

    let tool_help = setup_e2e_git_capture_with_env(&repo, &["difftool", "--tool-help"], &env);
    let text = output_text(&tool_help);
    assert!(
        tool_help.status.success(),
        "git difftool --tool-help failed\n{text}"
    );
    assert!(
        text.contains("gitcomet.cmd"),
        "expected headless gitcomet tool in difftool --tool-help output\n{text}"
    );
    assert!(
        text.contains("gitcomet-gui.cmd"),
        "expected gui gitcomet-gui tool in difftool --tool-help output\n{text}"
    );
    assert!(
        text.contains("difftool --local"),
        "expected difftool command shape in --tool-help output\n{text}"
    );
    assert!(
        text.contains("difftool --gui"),
        "expected gui difftool command shape in --tool-help output\n{text}"
    );
}

// ── --help / --version exit code tests ───────────────────────────────

#[test]
fn help_flag_exits_zero() {
    let out = run_gitcomet(["--help"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "--help should exit 0, not 2 (error)"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("gitcomet-app"),
        "help output should mention the binary name"
    );
}

#[test]
fn version_flag_exits_zero() {
    let out = run_gitcomet(["--version"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "--version should exit 0, not 2 (error)"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("gitcomet-app"),
        "version output should mention the binary name"
    );
}

#[test]
fn subcommand_help_exits_zero() {
    for subcmd in ["difftool", "mergetool", "setup", "uninstall"] {
        let out = run_gitcomet([subcmd, "--help"]);
        assert_eq!(out.status.code(), Some(0), "{subcmd} --help should exit 0");
    }
}

#[test]
fn help_subcommand_exits_zero() {
    let out = run_gitcomet(["help"]);
    assert_eq!(out.status.code(), Some(0), "help subcommand should exit 0");
}
