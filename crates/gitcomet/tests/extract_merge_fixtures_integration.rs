use gitcomet_core::process::background_command as no_window_command;
mod test_git_env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn apply_isolated_git_config_env(cmd: &mut Command) {
    test_git_env::apply(cmd);
    // Force deterministic git output for string assertions in tests.
    cmd.env("LC_ALL", "C");
    cmd.env("LANG", "C");
}

fn gitcomet_bin() -> PathBuf {
    for env_key in ["CARGO_BIN_EXE_gitcomet"] {
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
        "gitcomet binary path was not found. Tried CARGO_BIN_EXE_gitcomet and a fallback relative to current test executable"
    );
}

fn gitcomet_bin_from_current_exe() -> Option<PathBuf> {
    let test_exe = std::env::current_exe().ok()?;
    let deps_dir = test_exe.parent()?;
    let profile_dir = deps_dir.parent()?;
    let exe_suffix = std::env::consts::EXE_SUFFIX;

    for bin_name in ["gitcomet"] {
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
    no_window_command(gitcomet_bin())
        .args(args)
        .output()
        .expect("gitcomet command to run")
}

fn run_git_capture(repo: &Path, args: &[&str]) -> Output {
    let mut cmd = no_window_command("git");
    apply_isolated_git_config_env(&mut cmd);
    cmd.arg("-c")
        .arg("commit.gpgsign=false")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap_or_else(|e| panic!("failed to run git {:?}: {e}", args))
}

fn run_git(repo: &Path, args: &[&str]) {
    let output = run_git_capture(repo, args);
    assert!(
        output.status.success(),
        "git {:?} failed:\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn output_text(output: &Output) -> String {
    format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn configure_git_user(repo: &Path) {
    run_git(repo, &["config", "user.email", "test@example.com"]);
    run_git(repo, &["config", "user.name", "Test User"]);
}

fn count_suffix(dir: &Path, suffix: &str) -> usize {
    fs::read_dir(dir)
        .expect("read fixture output directory")
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str().map(ToOwned::to_owned))
        .filter(|name| name.ends_with(suffix))
        .count()
}

fn create_conflicting_merge_repo() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().expect("create temp dir");
    let repo = tmp.path();

    run_git(repo, &["init"]);
    run_git(repo, &["checkout", "-b", "main"]);
    configure_git_user(repo);

    fs::write(repo.join("a.txt"), "base line\n").expect("write base");
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["commit", "-m", "base"]);

    run_git(repo, &["checkout", "-b", "branch-a"]);
    fs::write(repo.join("a.txt"), "branch a change\n").expect("write branch-a");
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["commit", "-m", "branch-a change"]);

    run_git(repo, &["checkout", "main"]);
    run_git(repo, &["checkout", "-b", "branch-b"]);
    fs::write(repo.join("a.txt"), "branch b change\n").expect("write branch-b");
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["commit", "-m", "branch-b change"]);

    let merge = run_git_capture(repo, &["merge", "branch-a", "--no-edit"]);
    assert!(
        !merge.status.success(),
        "expected merge conflict while building fixture repo:\n{}",
        output_text(&merge)
    );

    fs::write(repo.join("a.txt"), "resolved merge result\n").expect("write resolution");
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["commit", "-m", "merge commit"]);

    tmp
}

#[test]
fn extract_merge_fixtures_e2e_writes_fixture_sets() {
    let repo = create_conflicting_merge_repo();
    let out = tempfile::tempdir().expect("create output dir");

    let output = run_gitcomet([
        OsString::from("extract-merge-fixtures"),
        OsString::from("--repo"),
        repo.path().as_os_str().to_owned(),
        OsString::from("--out"),
        out.path().as_os_str().to_owned(),
        OsString::from("--max-merges"),
        OsString::from("10"),
        OsString::from("--max-files-per-merge"),
        OsString::from("5"),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "expected exit 0\n{text}");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Extracted "),
        "expected extraction summary in stdout\n{text}"
    );
    assert!(
        stdout.contains("Wrote fixtures to"),
        "expected destination summary in stdout\n{text}"
    );

    let base_count = count_suffix(out.path(), "_base.txt");
    let contrib1_count = count_suffix(out.path(), "_contrib1.txt");
    let contrib2_count = count_suffix(out.path(), "_contrib2.txt");
    let expected_count = count_suffix(out.path(), "_expected_result.txt");

    assert!(
        base_count >= 1,
        "expected at least one extracted fixture\n{text}"
    );
    assert_eq!(
        contrib1_count, base_count,
        "contrib1 fixture count mismatch"
    );
    assert_eq!(
        contrib2_count, base_count,
        "contrib2 fixture count mismatch"
    );
    assert_eq!(
        expected_count, base_count,
        "expected fixture count mismatch"
    );
}

#[test]
fn extract_merge_fixtures_e2e_non_repo_exits_two() {
    let non_repo = tempfile::tempdir().expect("create non-repo dir");
    let out = tempfile::tempdir().expect("create output dir");

    let output = run_gitcomet([
        OsString::from("extract-merge-fixtures"),
        OsString::from("--repo"),
        non_repo.path().as_os_str().to_owned(),
        OsString::from("--out"),
        out.path().as_os_str().to_owned(),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(2), "expected exit 2\n{text}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("not a git repository"),
        "expected repository validation error\n{text}"
    );
}

#[test]
fn extract_merge_fixtures_e2e_rejects_zero_max_merges() {
    let repo = create_conflicting_merge_repo();
    let out = tempfile::tempdir().expect("create output dir");

    let output = run_gitcomet([
        OsString::from("extract-merge-fixtures"),
        OsString::from("--repo"),
        repo.path().as_os_str().to_owned(),
        OsString::from("--out"),
        out.path().as_os_str().to_owned(),
        OsString::from("--max-merges"),
        OsString::from("0"),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(2), "expected exit 2\n{text}");
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("Invalid --max-merges value '0': expected a positive integer."),
        "expected max-merges validation error\n{text}"
    );
}

#[test]
fn extract_merge_fixtures_e2e_rejects_zero_max_files_per_merge() {
    let repo = create_conflicting_merge_repo();
    let out = tempfile::tempdir().expect("create output dir");

    let output = run_gitcomet([
        OsString::from("extract-merge-fixtures"),
        OsString::from("--repo"),
        repo.path().as_os_str().to_owned(),
        OsString::from("--out"),
        out.path().as_os_str().to_owned(),
        OsString::from("--max-files-per-merge"),
        OsString::from("0"),
    ]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(2), "expected exit 2\n{text}");
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("Invalid --max-files-per-merge value '0': expected a positive integer."),
        "expected max-files-per-merge validation error\n{text}"
    );
}
