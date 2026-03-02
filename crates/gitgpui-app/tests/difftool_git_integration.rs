use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn gitgpui_bin() -> PathBuf {
    std::env::var_os("CARGO_BIN_EXE_gitgpui-app")
        .map(PathBuf::from)
        .expect("CARGO_BIN_EXE_gitgpui-app is not set for integration tests")
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn run_git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("git command to run");
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_git_capture(repo: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("git command to run")
}

fn run_git_capture_with_display(repo: &Path, args: &[&str], display: Option<&str>) -> Output {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(repo).args(args);
    if let Some(display) = display {
        cmd.env("DISPLAY", display);
    } else {
        cmd.env_remove("DISPLAY");
    }
    cmd.output().expect("git command to run")
}

fn run_git_capture_in(cwd: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("git command to run")
}

fn write_file(repo: &Path, rel: &str, contents: &str) {
    let path = repo.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent directories");
    }
    fs::write(path, contents).expect("write file");
}

fn write_bytes(repo: &Path, rel: &str, contents: &[u8]) {
    let path = repo.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent directories");
    }
    fs::write(path, contents).expect("write bytes");
}

fn init_repo(repo: &Path) {
    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
}

fn commit_all(repo: &Path, message: &str) {
    run_git(repo, &["add", "-A"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", message],
    );
}

fn git_head(repo: &Path) -> String {
    let output = run_git_capture(repo, &["rev-parse", "HEAD"]);
    assert!(
        output.status.success(),
        "git rev-parse HEAD failed\n{}",
        output_text(&output)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn gitgpui_difftool_cmd(marker: &str, force_exit: Option<i32>) -> String {
    let bin = gitgpui_bin();
    let bin_q = shell_quote(&bin.to_string_lossy());
    let mut cmd = format!(
        "echo TOOL={marker} >&2; if [ -n \"$MERGED\" ]; then {bin_q} difftool --local \"$LOCAL\" --remote \"$REMOTE\" --path \"$MERGED\"; else {bin_q} difftool --local \"$LOCAL\" --remote \"$REMOTE\"; fi"
    );
    if let Some(code) = force_exit {
        cmd.push_str(&format!("; exit {code}"));
    }
    cmd
}

fn configure_difftool_command(repo: &Path, tool: &str, cmd: &str) {
    let cmd_key = format!("difftool.{tool}.cmd");
    run_git(repo, &["config", &cmd_key, cmd]);
}

fn configure_difftool_trust_exit_code(repo: &Path, trust_exit_code: bool) {
    run_git(
        repo,
        &[
            "config",
            "difftool.trustExitCode",
            if trust_exit_code { "true" } else { "false" },
        ],
    );
}

fn configure_difftool_selection(
    repo: &Path,
    diff_tool: &str,
    diff_guitool: Option<&str>,
    gui_default: Option<&str>,
) {
    run_git(repo, &["config", "diff.tool", diff_tool]);
    if let Some(gui_tool) = diff_guitool {
        run_git(repo, &["config", "diff.guitool", gui_tool]);
    }
    if let Some(gui_default) = gui_default {
        run_git(repo, &["config", "difftool.guiDefault", gui_default]);
    }
    run_git(repo, &["config", "difftool.prompt", "false"]);
}

fn configure_gitgpui_difftool(repo: &Path) {
    configure_difftool_command(repo, "gitgpui", &gitgpui_difftool_cmd("gitgpui", None));
    configure_difftool_trust_exit_code(repo, true);
    configure_difftool_selection(repo, "gitgpui", None, None);
}

fn configure_kdiff3_path_override_to_gitgpui(repo: &Path) {
    let bin = gitgpui_bin();
    let bin_path = bin.to_string_lossy().to_string();
    run_git(repo, &["config", "diff.tool", "kdiff3"]);
    run_git(repo, &["config", "difftool.kdiff3.path", &bin_path]);
    run_git(repo, &["config", "difftool.trustExitCode", "true"]);
    run_git(repo, &["config", "difftool.prompt", "false"]);
}

fn configure_meld_path_override_to_gitgpui(repo: &Path) {
    let bin = gitgpui_bin();
    let bin_path = bin.to_string_lossy().to_string();
    run_git(repo, &["config", "diff.tool", "meld"]);
    run_git(repo, &["config", "difftool.meld.path", &bin_path]);
    run_git(repo, &["config", "difftool.trustExitCode", "true"]);
    run_git(repo, &["config", "difftool.prompt", "false"]);
}

fn output_text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn git_difftool_invokes_gitgpui_app_for_basic_diff() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_file(repo, "a.txt", "before\n");
    commit_all(repo, "base");

    write_file(repo, "a.txt", "after\n");
    configure_gitgpui_difftool(repo);

    let output = run_git_capture(repo, &["difftool", "--no-prompt", "--", "a.txt"]);
    let text = output_text(&output);
    assert!(output.status.success(), "git difftool failed\n{text}");
    assert!(
        text.contains("-before"),
        "missing removed line in output\n{text}"
    );
    assert!(
        text.contains("+after"),
        "missing added line in output\n{text}"
    );
}

#[test]
fn git_difftool_kdiff3_path_override_invokes_compat_mode() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_file(repo, "a.txt", "before\n");
    commit_all(repo, "base");
    write_file(repo, "a.txt", "after\n");

    configure_kdiff3_path_override_to_gitgpui(repo);

    let output = run_git_capture(repo, &["difftool", "--no-prompt", "--", "a.txt"]);
    let text = output_text(&output);
    assert!(
        output.status.success(),
        "expected kdiff3 path-override invocation to succeed with gitgpui compatibility parsing\n{text}"
    );
}

#[test]
fn git_difftool_meld_path_override_invokes_compat_mode() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_file(repo, "a.txt", "before\n");
    commit_all(repo, "base");
    write_file(repo, "a.txt", "after\n");

    configure_meld_path_override_to_gitgpui(repo);

    let output = run_git_capture(repo, &["difftool", "--no-prompt", "--", "a.txt"]);
    let text = output_text(&output);
    assert!(
        output.status.success(),
        "expected meld path-override invocation to succeed with gitgpui compatibility parsing\n{text}"
    );
    assert!(
        text.contains("-before") && text.contains("+after"),
        "expected meld path-override invocation to emit a diff\n{text}"
    );
}

#[test]
fn git_difftool_kdiff3_path_override_handles_spaced_unicode_path() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    let compat_path = "docs/spaced \u{65e5}\u{672c}\u{8a9e} file.txt";
    write_file(repo, compat_path, "before compat\n");
    commit_all(repo, "base");
    write_file(repo, compat_path, "after compat\n");

    configure_kdiff3_path_override_to_gitgpui(repo);

    let output = run_git_capture(repo, &["difftool", "--no-prompt", "--", compat_path]);
    let text = output_text(&output);
    assert!(
        output.status.success(),
        "expected kdiff3 path-override to handle spaced/unicode path\n{text}"
    );
    assert!(
        !text.contains("Invalid external"),
        "compat parser rejected spaced/unicode path\n{text}"
    );
    assert!(
        !text.contains("No such file or directory") && !text.contains("does not exist"),
        "expected kdiff3 path-override invocation to resolve spaced/unicode paths without file-resolution errors\n{text}"
    );
}

#[test]
fn git_difftool_meld_path_override_handles_spaced_unicode_path() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    let compat_path = "docs/spaced \u{65e5}\u{672c}\u{8a9e} file.txt";
    write_file(repo, compat_path, "before compat\n");
    commit_all(repo, "base");
    write_file(repo, compat_path, "after compat\n");

    configure_meld_path_override_to_gitgpui(repo);

    let output = run_git_capture(repo, &["difftool", "--no-prompt", "--", compat_path]);
    let text = output_text(&output);
    assert!(
        output.status.success(),
        "expected meld path-override to handle spaced/unicode path\n{text}"
    );
    assert!(
        !text.contains("Invalid external"),
        "compat parser rejected spaced/unicode path\n{text}"
    );
    assert!(
        text.contains("-before compat") && text.contains("+after compat"),
        "expected diff output for spaced/unicode path\n{text}"
    );
}

#[test]
fn git_difftool_handles_path_with_spaces() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_file(repo, "docs/spaced name.txt", "left side\n");
    commit_all(repo, "base");

    write_file(repo, "docs/spaced name.txt", "right side\n");
    configure_gitgpui_difftool(repo);

    let output = run_git_capture(
        repo,
        &["difftool", "--no-prompt", "--", "docs/spaced name.txt"],
    );
    let text = output_text(&output);
    assert!(
        output.status.success(),
        "git difftool failed for spaced path\n{text}"
    );
    assert!(
        text.contains("spaced name.txt"),
        "expected spaced filename in output\n{text}"
    );
    assert!(
        text.contains("-left side") && text.contains("+right side"),
        "missing expected line delta in output\n{text}"
    );
}

#[test]
fn git_difftool_handles_unicode_path() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    let unicode_path = "docs/\u{65e5}\u{672c}\u{8a9e}-\u{0444}\u{0430}\u{0439}\u{043b}.txt";
    write_file(repo, unicode_path, "left unicode side\n");
    commit_all(repo, "base");

    write_file(repo, unicode_path, "right unicode side\n");
    configure_gitgpui_difftool(repo);

    let output = run_git_capture(repo, &["difftool", "--no-prompt", "--", unicode_path]);
    let text = output_text(&output);
    assert!(
        output.status.success(),
        "git difftool failed for unicode path\n{text}"
    );
    assert!(
        text.contains("-left unicode side") && text.contains("+right unicode side"),
        "missing expected line delta in output\n{text}"
    );
}

#[test]
fn git_difftool_works_from_subdirectory() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_file(repo, "nested/deeper/file.txt", "old value\n");
    commit_all(repo, "base");

    write_file(repo, "nested/deeper/file.txt", "new value\n");
    configure_gitgpui_difftool(repo);

    let subdir = repo.join("nested/deeper");
    let output = run_git_capture_in(&subdir, &["difftool", "--no-prompt", "--", "file.txt"]);
    let text = output_text(&output);
    assert!(
        output.status.success(),
        "git difftool failed from subdirectory\n{text}"
    );
    assert!(
        text.contains("-old value") && text.contains("+new value"),
        "missing expected delta for subdirectory invocation\n{text}"
    );
}

#[test]
fn git_difftool_dir_diff_mode_works() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_file(repo, "tracked.txt", "one\n");
    commit_all(repo, "base");

    write_file(repo, "tracked.txt", "two\n");
    configure_gitgpui_difftool(repo);

    let output = run_git_capture(repo, &["difftool", "--dir-diff", "--no-prompt"]);
    let text = output_text(&output);
    assert!(
        output.status.success(),
        "git difftool --dir-diff failed\n{text}"
    );
    assert!(
        text.contains("tracked.txt"),
        "expected tracked filename in dir-diff output\n{text}"
    );
}

#[test]
fn git_difftool_dir_diff_handles_spaced_unicode_path() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    let tricky_path = "nested/spaced \u{65e5}\u{672c}\u{8a9e} file.txt";
    write_file(repo, tricky_path, "before line\n");
    commit_all(repo, "base");

    write_file(repo, tricky_path, "after line\n");
    configure_gitgpui_difftool(repo);

    let output = run_git_capture(repo, &["difftool", "--dir-diff", "--no-prompt"]);
    let text = output_text(&output);
    assert!(
        output.status.success(),
        "git difftool --dir-diff failed for spaced/unicode path\n{text}"
    );
    assert!(
        text.contains("spaced") && text.contains("file.txt"),
        "expected spaced/unicode filename in dir-diff output\n{text}"
    );
    assert!(
        text.contains("-before line") && text.contains("+after line"),
        "missing expected line delta in dir-diff output\n{text}"
    );
}

#[test]
fn git_difftool_dir_diff_mode_works_from_subdirectory() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_file(repo, "nested/deeper/tracked.txt", "tracked old\n");
    write_file(repo, "docs/selected.txt", "selected old\n");
    commit_all(repo, "base");

    write_file(repo, "nested/deeper/tracked.txt", "tracked new\n");
    write_file(repo, "docs/selected.txt", "selected new\n");
    configure_gitgpui_difftool(repo);

    let subdir = repo.join("nested/deeper");
    let output = run_git_capture_in(&subdir, &["difftool", "--dir-diff", "--no-prompt"]);
    let text = output_text(&output);
    assert!(
        output.status.success(),
        "git difftool --dir-diff failed from subdirectory\n{text}"
    );
    assert!(
        text.contains("tracked.txt"),
        "expected nested tracked path in dir-diff output\n{text}"
    );
    assert!(
        text.contains("-tracked old") && text.contains("+tracked new"),
        "missing expected tracked-file delta in dir-diff output\n{text}"
    );
}

#[test]
fn git_difftool_dir_diff_pathspec_from_subdirectory_limits_to_selected_path() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_file(repo, "nested/deeper/tracked.txt", "tracked old\n");
    write_file(repo, "docs/selected.txt", "selected old\n");
    commit_all(repo, "base");

    write_file(repo, "nested/deeper/tracked.txt", "tracked new\n");
    write_file(repo, "docs/selected.txt", "selected new\n");
    configure_gitgpui_difftool(repo);

    let subdir = repo.join("nested/deeper");
    let output = run_git_capture_in(
        &subdir,
        &[
            "difftool",
            "--dir-diff",
            "--no-prompt",
            "--",
            "../../docs/selected.txt",
        ],
    );
    let text = output_text(&output);
    assert!(
        output.status.success(),
        "git difftool --dir-diff pathspec failed from subdirectory\n{text}"
    );
    assert!(
        text.contains("selected.txt"),
        "expected selected path in dir-diff pathspec output\n{text}"
    );
    assert!(
        text.contains("-selected old") && text.contains("+selected new"),
        "missing expected selected-path delta in dir-diff pathspec output\n{text}"
    );
    assert!(
        !text.contains("tracked.txt"),
        "did not expect unselected tracked path in dir-diff pathspec output\n{text}"
    );
}

#[test]
fn git_difftool_pathspec_limits_invocation_to_selected_path() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_file(repo, "docs/selected.txt", "selected old\n");
    write_file(repo, "docs/other.txt", "other old\n");
    commit_all(repo, "base");

    write_file(repo, "docs/selected.txt", "selected new\n");
    write_file(repo, "docs/other.txt", "other new\n");
    configure_gitgpui_difftool(repo);

    let output = run_git_capture(
        repo,
        &["difftool", "--no-prompt", "--", "docs/selected.txt"],
    );
    let text = output_text(&output);
    assert!(output.status.success(), "git difftool failed\n{text}");
    assert!(
        text.contains("selected.txt"),
        "expected selected path to be diffed\n{text}"
    );
    assert!(
        text.contains("-selected old") && text.contains("+selected new"),
        "expected selected path line delta in output\n{text}"
    );
    assert!(
        !text.contains("other.txt") && !text.contains("other old") && !text.contains("other new"),
        "expected pathspec to exclude non-selected path\n{text}"
    );
}

#[test]
fn git_difftool_handles_binary_content_change() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_bytes(repo, "blob.bin", &[0x00, 0x01, 0x02, 0x03, 0x04]);
    commit_all(repo, "base");

    write_bytes(repo, "blob.bin", &[0x00, 0x01, 0xFF, 0x03, 0x04]);
    configure_gitgpui_difftool(repo);

    let output = run_git_capture(repo, &["difftool", "--no-prompt", "--", "blob.bin"]);
    let text = output_text(&output);
    assert!(
        output.status.success(),
        "git difftool failed for binary content\n{text}"
    );
    assert!(
        text.contains("Binary files")
            || text.contains("GIT binary patch")
            || text.contains("blob.bin"),
        "expected binary diff signal in output\n{text}"
    );
}

#[test]
fn git_difftool_handles_non_utf8_content_change() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_bytes(repo, "data/non_utf8.dat", b"prefix\n\xFF\n");
    commit_all(repo, "base");

    write_bytes(repo, "data/non_utf8.dat", b"prefix\n\xFE\n");
    configure_gitgpui_difftool(repo);

    let output = run_git_capture(
        repo,
        &["difftool", "--no-prompt", "--", "data/non_utf8.dat"],
    );
    let text = output_text(&output);
    assert!(
        output.status.success(),
        "git difftool failed for non-UTF8 content\n{text}"
    );
    assert!(
        !text.trim().is_empty(),
        "expected diff output for non-UTF8 content"
    );
}

#[test]
fn git_difftool_gui_default_auto_prefers_gui_tool_when_display_set() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_file(repo, "a.txt", "before\n");
    commit_all(repo, "base");
    write_file(repo, "a.txt", "after\n");

    configure_difftool_command(repo, "cli", &gitgpui_difftool_cmd("cli", None));
    configure_difftool_command(repo, "gui", &gitgpui_difftool_cmd("gui", None));
    configure_difftool_trust_exit_code(repo, true);
    configure_difftool_selection(repo, "cli", Some("gui"), Some("auto"));

    let output = run_git_capture_with_display(
        repo,
        &["difftool", "--no-prompt", "--", "a.txt"],
        Some(":99"),
    );
    let text = output_text(&output);
    assert!(output.status.success(), "git difftool failed\n{text}");
    assert!(
        text.contains("TOOL=gui"),
        "expected gui tool selection with DISPLAY set\n{text}"
    );
}

#[test]
fn git_difftool_gui_default_auto_prefers_cli_tool_without_display() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_file(repo, "a.txt", "before\n");
    commit_all(repo, "base");
    write_file(repo, "a.txt", "after\n");

    configure_difftool_command(repo, "cli", &gitgpui_difftool_cmd("cli", None));
    configure_difftool_command(repo, "gui", &gitgpui_difftool_cmd("gui", None));
    configure_difftool_trust_exit_code(repo, true);
    configure_difftool_selection(repo, "cli", Some("gui"), Some("Auto"));

    let output =
        run_git_capture_with_display(repo, &["difftool", "--no-prompt", "--", "a.txt"], None);
    let text = output_text(&output);
    assert!(output.status.success(), "git difftool failed\n{text}");
    assert!(
        text.contains("TOOL=cli"),
        "expected cli tool selection without DISPLAY\n{text}"
    );
}

#[test]
fn git_difftool_gui_default_true_prefers_gui_tool_without_display() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_file(repo, "a.txt", "before\n");
    commit_all(repo, "base");
    write_file(repo, "a.txt", "after\n");

    configure_difftool_command(repo, "cli", &gitgpui_difftool_cmd("cli", None));
    configure_difftool_command(repo, "gui", &gitgpui_difftool_cmd("gui", None));
    configure_difftool_trust_exit_code(repo, true);
    configure_difftool_selection(repo, "cli", Some("gui"), Some("true"));

    let output =
        run_git_capture_with_display(repo, &["difftool", "--no-prompt", "--", "a.txt"], None);
    let text = output_text(&output);
    assert!(output.status.success(), "git difftool failed\n{text}");
    assert!(
        text.contains("TOOL=gui"),
        "expected gui tool selection when guiDefault=true\n{text}"
    );
}

#[test]
fn git_difftool_gui_default_false_prefers_cli_tool_with_display() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_file(repo, "a.txt", "before\n");
    commit_all(repo, "base");
    write_file(repo, "a.txt", "after\n");

    configure_difftool_command(repo, "cli", &gitgpui_difftool_cmd("cli", None));
    configure_difftool_command(repo, "gui", &gitgpui_difftool_cmd("gui", None));
    configure_difftool_trust_exit_code(repo, true);
    configure_difftool_selection(repo, "cli", Some("gui"), Some("false"));

    let output = run_git_capture_with_display(
        repo,
        &["difftool", "--no-prompt", "--", "a.txt"],
        Some(":99"),
    );
    let text = output_text(&output);
    assert!(output.status.success(), "git difftool failed\n{text}");
    assert!(
        text.contains("TOOL=cli"),
        "expected regular tool selection when guiDefault=false\n{text}"
    );
}

#[test]
fn git_difftool_gui_flag_overrides_selection() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_file(repo, "a.txt", "before\n");
    commit_all(repo, "base");
    write_file(repo, "a.txt", "after\n");

    configure_difftool_command(repo, "cli", &gitgpui_difftool_cmd("cli", None));
    configure_difftool_command(repo, "gui", &gitgpui_difftool_cmd("gui", None));
    configure_difftool_trust_exit_code(repo, true);
    configure_difftool_selection(repo, "cli", Some("gui"), Some("false"));

    let output = run_git_capture_with_display(
        repo,
        &["difftool", "--gui", "--no-prompt", "--", "a.txt"],
        None,
    );
    let text = output_text(&output);
    assert!(output.status.success(), "git difftool failed\n{text}");
    assert!(
        text.contains("TOOL=gui"),
        "expected --gui to force gui tool selection\n{text}"
    );
}

#[test]
fn git_difftool_no_gui_flag_overrides_gui_default_true() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_file(repo, "a.txt", "before\n");
    commit_all(repo, "base");
    write_file(repo, "a.txt", "after\n");

    configure_difftool_command(repo, "cli", &gitgpui_difftool_cmd("cli", None));
    configure_difftool_command(repo, "gui", &gitgpui_difftool_cmd("gui", None));
    configure_difftool_trust_exit_code(repo, true);
    configure_difftool_selection(repo, "cli", Some("gui"), Some("true"));

    let output = run_git_capture_with_display(
        repo,
        &["difftool", "--no-gui", "--no-prompt", "--", "a.txt"],
        Some(":99"),
    );
    let text = output_text(&output);
    assert!(output.status.success(), "git difftool failed\n{text}");
    assert!(
        text.contains("TOOL=cli"),
        "expected --no-gui to force regular tool selection\n{text}"
    );
}

#[test]
fn git_difftool_gui_fallback_when_no_guitool_configured() {
    // When --gui is requested but only diff.tool is configured, git difftool
    // should fall back to the regular tool selection.
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_file(repo, "a.txt", "before\n");
    commit_all(repo, "base");
    write_file(repo, "a.txt", "after\n");

    configure_difftool_command(repo, "cli", &gitgpui_difftool_cmd("cli", None));
    configure_difftool_trust_exit_code(repo, true);
    configure_difftool_selection(repo, "cli", None, Some("false"));

    let output = run_git_capture_with_display(
        repo,
        &["difftool", "--gui", "--no-prompt", "--", "a.txt"],
        Some(":99"),
    );
    let text = output_text(&output);
    assert!(output.status.success(), "git difftool failed\n{text}");
    assert!(
        text.contains("TOOL=cli"),
        "expected fallback to diff.tool when no diff.guitool configured\n{text}"
    );
}

#[test]
fn git_difftool_gui_default_true_fallback_when_no_guitool_configured() {
    // Even with guiDefault=true, git difftool should fall back to diff.tool
    // if no diff.guitool is configured.
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_file(repo, "a.txt", "before\n");
    commit_all(repo, "base");
    write_file(repo, "a.txt", "after\n");

    configure_difftool_command(repo, "cli", &gitgpui_difftool_cmd("cli", None));
    configure_difftool_trust_exit_code(repo, true);
    configure_difftool_selection(repo, "cli", None, Some("true"));

    let output = run_git_capture_with_display(
        repo,
        &["difftool", "--no-prompt", "--", "a.txt"],
        Some(":99"),
    );
    let text = output_text(&output);
    assert!(output.status.success(), "git difftool failed\n{text}");
    assert!(
        text.contains("TOOL=cli"),
        "expected fallback to diff.tool with guiDefault=true and no diff.guitool\n{text}"
    );
}

#[test]
fn git_difftool_honors_tool_trust_exit_code_false() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_file(repo, "a.txt", "before\n");
    commit_all(repo, "base");
    write_file(repo, "a.txt", "after\n");

    configure_difftool_command(repo, "failer", "echo TOOL=failer >&2; exit 7");
    configure_difftool_trust_exit_code(repo, false);
    configure_difftool_selection(repo, "failer", None, None);

    let output = run_git_capture(repo, &["difftool", "--no-prompt", "--", "a.txt"]);
    let text = output_text(&output);
    assert!(
        output.status.success(),
        "expected trustExitCode=false to ignore tool failure\n{text}"
    );
}

#[test]
fn git_difftool_honors_tool_trust_exit_code_true() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_file(repo, "a.txt", "before\n");
    commit_all(repo, "base");
    write_file(repo, "a.txt", "after\n");

    configure_difftool_command(repo, "failer", "echo TOOL=failer >&2; exit 7");
    configure_difftool_trust_exit_code(repo, true);
    configure_difftool_selection(repo, "failer", None, None);

    let output = run_git_capture(repo, &["difftool", "--no-prompt", "--", "a.txt"]);
    let text = output_text(&output);
    assert!(
        !output.status.success(),
        "expected trustExitCode=true to propagate tool failure\n{text}"
    );
}

#[test]
fn git_difftool_trust_exit_code_flag_overrides_config() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_file(repo, "a.txt", "before\n");
    commit_all(repo, "base");
    write_file(repo, "a.txt", "after\n");

    configure_difftool_command(repo, "failer", "echo TOOL=failer >&2; exit 7");
    configure_difftool_trust_exit_code(repo, false);
    configure_difftool_selection(repo, "failer", None, None);

    let forced_trust = run_git_capture(
        repo,
        &[
            "difftool",
            "--no-prompt",
            "--trust-exit-code",
            "--",
            "a.txt",
        ],
    );
    let forced_trust_text = output_text(&forced_trust);
    assert!(
        !forced_trust.status.success(),
        "expected --trust-exit-code to force failure propagation\n{forced_trust_text}"
    );

    configure_difftool_trust_exit_code(repo, true);
    let forced_no_trust = run_git_capture(
        repo,
        &[
            "difftool",
            "--no-prompt",
            "--no-trust-exit-code",
            "--",
            "a.txt",
        ],
    );
    let forced_no_trust_text = output_text(&forced_no_trust);
    assert!(
        forced_no_trust.status.success(),
        "expected --no-trust-exit-code to ignore failure\n{forced_no_trust_text}"
    );
}

#[test]
fn git_difftool_shows_submodule_gitlink_change() {
    // When only a submodule gitlink changes, git difftool passes temporary
    // files containing "Subproject commit <sha>" lines to the external tool.
    // Verify that GitGpui surfaces both old and new commit pointers.
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    let sub_repo = tmp.path().join("subrepo");
    fs::create_dir_all(&repo).expect("create main repo directory");
    fs::create_dir_all(&sub_repo).expect("create submodule repo directory");

    init_repo(&repo);
    init_repo(&sub_repo);

    write_file(&sub_repo, "sub.txt", "submodule v1\n");
    commit_all(&sub_repo, "submodule: v1");
    let old_commit = git_head(&sub_repo);

    let sub_url = sub_repo.to_string_lossy().to_string();
    run_git(
        &repo,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            &sub_url,
            "submod",
        ],
    );
    commit_all(&repo, "add submodule");

    write_file(&sub_repo, "sub.txt", "submodule v2\n");
    commit_all(&sub_repo, "submodule: v2");
    let new_commit = git_head(&sub_repo);

    run_git(&repo.join("submod"), &["fetch"]);
    run_git(&repo.join("submod"), &["checkout", &new_commit]);

    configure_gitgpui_difftool(&repo);

    let output = run_git_capture(&repo, &["difftool", "--no-prompt", "--", "submod"]);
    let text = output_text(&output);
    assert!(
        output.status.success(),
        "git difftool failed for submodule gitlink change\n{text}"
    );
    assert!(
        text.contains("Subproject commit"),
        "expected submodule gitlink content in difftool output\n{text}"
    );
    assert!(
        text.contains(&old_commit),
        "expected old submodule commit in difftool output\n{text}"
    );
    assert!(
        text.contains(&new_commit),
        "expected new submodule commit in difftool output\n{text}"
    );
}

// ── Symlink diff ─────────────────────────────────────────────────────

#[test]
fn git_difftool_shows_symlink_target_change() {
    // When a symlink target changes, git difftool shows the diff of
    // the symlink targets (short text strings).
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    std::os::unix::fs::symlink("original_target", repo.join("link")).expect("create symlink");
    commit_all(repo, "base: add symlink");

    // Change the symlink target.
    fs::remove_file(repo.join("link")).unwrap();
    std::os::unix::fs::symlink("new_target", repo.join("link")).expect("create symlink");

    configure_gitgpui_difftool(repo);

    let output = run_git_capture(repo, &["difftool", "--no-prompt", "--", "link"]);
    let text = output_text(&output);

    // Git shows symlink targets as file content to the difftool.
    // Our tool should produce a diff between "original_target" and "new_target".
    assert!(
        output.status.success(),
        "git difftool failed for symlink\n{text}"
    );
    assert!(
        text.contains("original_target") || text.contains("new_target"),
        "expected symlink target content in difftool output\n{text}"
    );
}

#[test]
fn git_difftool_tool_help_lists_gitgpui_tool() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    configure_gitgpui_difftool(repo);

    let output = run_git_capture(repo, &["difftool", "--tool-help"]);
    let text = output_text(&output);
    assert!(
        output.status.success(),
        "git difftool --tool-help failed\n{text}"
    );
    assert!(
        text.contains("gitgpui"),
        "expected gitgpui tool name in --tool-help output\n{text}"
    );
}

#[test]
fn git_difftool_absent_tool_reports_cmd_not_set_error() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_file(repo, "a.txt", "before\n");
    commit_all(repo, "base");

    write_file(repo, "a.txt", "after\n");
    run_git(repo, &["config", "difftool.prompt", "false"]);

    let output = run_git_capture(
        repo,
        &["difftool", "--no-prompt", "--tool", "absent", "--", "a.txt"],
    );
    let text = output_text(&output);
    assert!(
        !output.status.success(),
        "expected git difftool --tool absent to fail\n{text}"
    );
    assert!(
        text.contains("cmd not set for tool 'absent'"),
        "expected missing-tool command error text\n{text}"
    );
}
