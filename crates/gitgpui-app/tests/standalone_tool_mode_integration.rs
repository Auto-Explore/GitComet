use std::ffi::{OsStr, OsString};
use std::fs;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn gitgpui_bin() -> PathBuf {
    std::env::var_os("CARGO_BIN_EXE_gitgpui-app")
        .map(PathBuf::from)
        .expect("CARGO_BIN_EXE_gitgpui-app is not set for integration tests")
}

fn run_gitgpui<I, S>(args: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(gitgpui_bin())
        .args(args)
        .output()
        .expect("gitgpui-app command to run")
}

fn run_gitgpui_in_dir<I, S>(dir: &Path, args: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(gitgpui_bin())
        .current_dir(dir)
        .args(args)
        .output()
        .expect("gitgpui-app command to run")
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

fn write_file(path: &Path, contents: &str) {
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

    let output = run_gitgpui([
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

    let output = run_gitgpui([
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
fn standalone_mergetool_marker_size_flag_controls_marker_width() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.txt");
    let local = dir.path().join("local.txt");
    let remote = dir.path().join("remote.txt");
    let merged = dir.path().join("merged.txt");

    write_file(&base, "line\n");
    write_file(&local, "ours\n");
    write_file(&remote, "theirs\n");

    let output = run_gitgpui([
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
fn standalone_mergetool_invalid_path_exits_two() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.txt");
    let local = dir.path().join("local.txt");
    let missing_remote = dir.path().join("missing_remote.txt");
    let merged = dir.path().join("merged.txt");

    write_file(&base, "line\n");
    write_file(&local, "line\n");

    let output = run_gitgpui([
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

#[test]
fn standalone_difftool_changed_files_exits_zero_and_prints_diff() {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("left.txt");
    let remote = dir.path().join("right.txt");

    write_file(&local, "left\n");
    write_file(&remote, "right\n");

    let output = run_gitgpui([
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
fn standalone_compat_difftool_accepts_meld_style_label_flags() {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("left.txt");
    let remote = dir.path().join("right.txt");

    write_file(&local, "left\n");
    write_file(&remote, "right\n");

    let output = run_gitgpui([
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
fn standalone_compat_mergetool_meld_label_order_maps_to_local_base_remote() {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("local.txt");
    let base = dir.path().join("base.txt");
    let remote = dir.path().join("remote.txt");
    let merged = dir.path().join("merged.txt");

    write_file(&base, "line\n");
    write_file(&local, "local change\n");
    write_file(&remote, "remote change\n");

    let output = run_gitgpui([
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
fn standalone_difftool_missing_input_exits_two() {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("left.txt");
    let missing_remote = dir.path().join("missing.txt");
    write_file(&local, "left\n");

    let output = run_gitgpui([
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

// ── Setup subcommand tests ───────────────────────────────────────────

#[test]
fn setup_dry_run_prints_commands_without_writing() {
    let output = run_gitgpui([OsString::from("setup"), OsString::from("--dry-run")]);

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "expected exit 0\n{text}");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Dry run"),
        "expected dry-run header\n{text}"
    );
    assert!(
        stdout.contains("git config --global merge.tool"),
        "expected merge.tool\n{text}"
    );
    assert!(
        stdout.contains("git config --global diff.tool"),
        "expected diff.tool\n{text}"
    );
    assert!(
        stdout.contains("mergetool.gitgpui.cmd"),
        "expected mergetool cmd\n{text}"
    );
    assert!(
        stdout.contains("difftool.gitgpui.cmd"),
        "expected difftool cmd\n{text}"
    );
    assert!(
        stdout.contains("mergetool.trustExitCode"),
        "expected mergetool.trustExitCode\n{text}"
    );
    assert!(
        stdout.contains("mergetool.gitgpui.trustExitCode"),
        "expected mergetool.gitgpui.trustExitCode\n{text}"
    );
    assert!(
        stdout.contains("difftool.trustExitCode"),
        "expected difftool.trustExitCode\n{text}"
    );
}

#[test]
fn setup_dry_run_local_uses_local_scope() {
    let output = run_gitgpui([
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
    let dir = tempfile::tempdir().unwrap();

    let init = Command::new("git")
        .args(["init", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(init.status.success(), "git init failed");

    let output = run_gitgpui_in_dir(
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

    let merge_cmd = git_config_get(dir.path(), "mergetool.gitgpui.cmd")
        .expect("mergetool cmd should be configured by dry-run commands");
    assert!(merge_cmd.contains("$BASE"), "expected literal $BASE in cmd");
    assert!(
        merge_cmd.contains("$LOCAL"),
        "expected literal $LOCAL in cmd"
    );
    assert!(
        merge_cmd.contains("$REMOTE"),
        "expected literal $REMOTE in cmd"
    );
    assert!(
        merge_cmd.contains("$MERGED"),
        "expected literal $MERGED in cmd"
    );

    let diff_cmd = git_config_get(dir.path(), "difftool.gitgpui.cmd")
        .expect("difftool cmd should be configured by dry-run commands");
    assert!(
        diff_cmd.contains("$LOCAL"),
        "expected literal $LOCAL in cmd"
    );
    assert!(
        diff_cmd.contains("$REMOTE"),
        "expected literal $REMOTE in cmd"
    );
    assert!(
        diff_cmd.contains("$MERGED"),
        "expected literal $MERGED in cmd"
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

    let output = run_gitgpui_in_dir(
        dir.path(),
        [OsString::from("setup"), OsString::from("--local")],
    );

    let text = output_text(&output);
    assert_eq!(output.status.code(), Some(0), "expected exit 0\n{text}");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Configured gitgpui as local diff/merge tool"),
        "{text}"
    );

    // Verify key config entries were written.
    assert_eq!(
        git_config_get(dir.path(), "merge.tool").as_deref(),
        Some("gitgpui"),
        "merge.tool not set"
    );
    assert_eq!(
        git_config_get(dir.path(), "diff.tool").as_deref(),
        Some("gitgpui"),
        "diff.tool not set"
    );
    assert_eq!(
        git_config_get(dir.path(), "mergetool.trustExitCode").as_deref(),
        Some("true"),
        "mergetool.trustExitCode not set"
    );
    assert_eq!(
        git_config_get(dir.path(), "mergetool.gitgpui.trustExitCode").as_deref(),
        Some("true"),
        "mergetool.gitgpui.trustExitCode not set"
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
        Some("gitgpui"),
        "merge.guitool not set"
    );
    assert_eq!(
        git_config_get(dir.path(), "mergetool.guiDefault").as_deref(),
        Some("auto"),
        "mergetool.guiDefault not set"
    );

    // Verify the cmd contains the binary path and proper variable quoting.
    let merge_cmd =
        git_config_get(dir.path(), "mergetool.gitgpui.cmd").expect("mergetool cmd should be set");
    assert!(merge_cmd.contains("$BASE"), "merge cmd missing $BASE");
    assert!(merge_cmd.contains("$LOCAL"), "merge cmd missing $LOCAL");
    assert!(merge_cmd.contains("$REMOTE"), "merge cmd missing $REMOTE");
    assert!(merge_cmd.contains("$MERGED"), "merge cmd missing $MERGED");

    let diff_cmd =
        git_config_get(dir.path(), "difftool.gitgpui.cmd").expect("difftool cmd should be set");
    assert!(diff_cmd.contains("$LOCAL"), "diff cmd missing $LOCAL");
    assert!(diff_cmd.contains("$REMOTE"), "diff cmd missing $REMOTE");
}
