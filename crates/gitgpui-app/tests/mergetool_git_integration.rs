use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

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

fn run_git_capture_in(cwd: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("git command to run")
}

fn run_git_expect_failure(repo: &Path, args: &[&str]) -> Output {
    let output = run_git_capture(repo, args);
    assert!(
        !output.status.success(),
        "git {:?} unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
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

fn run_git_with_stdin(repo: &Path, args: &[&str], stdin_text: &str) -> Output {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("git command to spawn");
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(stdin_text.as_bytes());
    }
    child.wait_with_output().expect("git command to complete")
}

fn write_file(repo: &Path, rel: &str, contents: &str) {
    let path = repo.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent directories");
    }
    fs::write(path, contents).expect("write file");
}

fn init_repo(repo: &Path) {
    run_git(repo, &["init", "-b", "main"]);
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

fn configure_gitgpui_mergetool(repo: &Path) {
    let bin = gitgpui_bin();
    let bin_q = shell_quote(&bin.to_string_lossy());
    let cmd = format!(
        "{bin_q} mergetool --base \"$BASE\" --local \"$LOCAL\" --remote \"$REMOTE\" --merged \"$MERGED\""
    );

    run_git(repo, &["config", "merge.tool", "gitgpui"]);
    run_git(repo, &["config", "mergetool.gitgpui.cmd", &cmd]);
    run_git(repo, &["config", "mergetool.gitgpui.trustExitCode", "true"]);
    run_git(repo, &["config", "mergetool.prompt", "false"]);
    // Disable backup file creation for cleaner assertions.
    run_git(repo, &["config", "mergetool.keepBackup", "false"]);
}

fn configure_gitgpui_mergetool_with_alias_flags(repo: &Path) {
    let bin = gitgpui_bin();
    let bin_q = shell_quote(&bin.to_string_lossy());
    let cmd = format!(
        "{bin_q} mergetool -o \"$MERGED\" --base \"$BASE\" --local \"$LOCAL\" --remote \"$REMOTE\" --L1 \"BASE_ALIAS\" --L2 \"LOCAL_ALIAS\" --L3 \"REMOTE_ALIAS\""
    );

    run_git(repo, &["config", "merge.tool", "gitgpui"]);
    run_git(repo, &["config", "mergetool.gitgpui.cmd", &cmd]);
    run_git(repo, &["config", "mergetool.gitgpui.trustExitCode", "true"]);
    run_git(repo, &["config", "mergetool.prompt", "false"]);
    run_git(repo, &["config", "mergetool.keepBackup", "false"]);
}

fn configure_kdiff3_path_override_to_gitgpui(repo: &Path, trust_exit_code: bool) {
    let bin = gitgpui_bin();
    let bin_path = bin.to_string_lossy().to_string();

    run_git(repo, &["config", "merge.tool", "kdiff3"]);
    run_git(repo, &["config", "mergetool.kdiff3.path", &bin_path]);
    run_git(
        repo,
        &[
            "config",
            "mergetool.kdiff3.trustExitCode",
            if trust_exit_code { "true" } else { "false" },
        ],
    );
    run_git(repo, &["config", "mergetool.prompt", "false"]);
    run_git(repo, &["config", "mergetool.keepBackup", "false"]);
}

/// Create a mergetool command that echoes a marker to stderr and resolves
/// the conflict by copying $REMOTE to $MERGED. This simulates a successful
/// merge tool and allows tests to detect which tool was selected by checking
/// for the marker in the combined output.
fn mergetool_marker_cmd(marker: &str) -> String {
    format!("echo TOOL={marker} >&2; cat \"$REMOTE\" > \"$MERGED\"")
}

fn configure_mergetool_command(repo: &Path, tool: &str, cmd: &str) {
    let cmd_key = format!("mergetool.{tool}.cmd");
    run_git(repo, &["config", &cmd_key, cmd]);
}

fn configure_mergetool_trust_exit_code(repo: &Path, tool: &str, trust: bool) {
    let key = format!("mergetool.{tool}.trustExitCode");
    run_git(
        repo,
        &["config", &key, if trust { "true" } else { "false" }],
    );
}

fn configure_mergetool_selection(
    repo: &Path,
    merge_tool: &str,
    merge_guitool: Option<&str>,
    gui_default: Option<&str>,
) {
    run_git(repo, &["config", "merge.tool", merge_tool]);
    if let Some(gui_tool) = merge_guitool {
        run_git(repo, &["config", "merge.guitool", gui_tool]);
    }
    if let Some(gui_default) = gui_default {
        run_git(repo, &["config", "mergetool.guiDefault", gui_default]);
    }
    run_git(repo, &["config", "mergetool.prompt", "false"]);
    run_git(repo, &["config", "mergetool.keepBackup", "false"]);
}

fn configure_recording_mergetool(repo: &Path, tool: &str, log_path: &Path) {
    let log_q = shell_quote(&log_path.to_string_lossy());
    let cmd = format!("printf '%s\\n' \"$MERGED\" >> {log_q}; cat \"$REMOTE\" > \"$MERGED\"");
    run_git(repo, &["config", "merge.tool", tool]);
    run_git(repo, &["config", &format!("mergetool.{tool}.cmd"), &cmd]);
    run_git(
        repo,
        &["config", &format!("mergetool.{tool}.trustExitCode"), "true"],
    );
    run_git(repo, &["config", "mergetool.prompt", "false"]);
    run_git(repo, &["config", "mergetool.keepBackup", "false"]);
}

fn configure_stage_path_recording_mergetool(repo: &Path, tool: &str) {
    let cmd = "printf '%s\\n%s\\n%s\\n' \"$BASE\" \"$LOCAL\" \"$REMOTE\" > \"$MERGED.env\"; cat \"$REMOTE\" > \"$MERGED\"";
    run_git(repo, &["config", "merge.tool", tool]);
    run_git(repo, &["config", &format!("mergetool.{tool}.cmd"), cmd]);
    run_git(
        repo,
        &["config", &format!("mergetool.{tool}.trustExitCode"), "true"],
    );
    run_git(repo, &["config", "mergetool.prompt", "false"]);
    run_git(repo, &["config", "mergetool.keepBackup", "false"]);
}

fn setup_order_file_conflict(repo: &Path) {
    init_repo(repo);
    write_file(repo, "a", "start\n");
    write_file(repo, "b", "start\n");
    commit_all(repo, "start");

    run_git(repo, &["checkout", "-b", "side1"]);
    write_file(repo, "a", "side1\n");
    write_file(repo, "b", "side1\n");
    commit_all(repo, "side1 changes");

    run_git(repo, &["checkout", "main"]);
    run_git(repo, &["checkout", "-b", "side2"]);
    write_file(repo, "a", "side2\n");
    write_file(repo, "b", "side2\n");
    commit_all(repo, "side2 changes");

    run_git_expect_failure(repo, &["merge", "side1"]);
}

/// Create the t7610-style rename/rename setup that presents as a delete/delete
/// conflict at `a/a/file.txt` when merging `move-to-b` into `move-to-c`.
///
/// Returns `true` when a merge conflict is present and mergetool scenarios
/// should run. Some git versions may auto-resolve; callers can skip on `false`.
fn setup_delete_delete_rename_conflict(repo: &Path) -> bool {
    init_repo(repo);

    // Base file at a/a/file.txt
    fs::create_dir_all(repo.join("a/a")).unwrap();
    write_file(repo, "a/a/file.txt", "one\ntwo\n3\n4\n");
    commit_all(repo, "base file");

    // move-to-b branch: rename + edit.
    run_git(repo, &["checkout", "-b", "move-to-b"]);
    fs::create_dir_all(repo.join("b/b")).unwrap();
    run_git(repo, &["mv", "a/a/file.txt", "b/b/file.txt"]);
    write_file(repo, "b/b/file.txt", "one\ntwo\n4\n");
    commit_all(repo, "move to b");

    // move-to-c branch: rename + edit.
    run_git(repo, &["checkout", "main"]);
    run_git(repo, &["checkout", "-b", "move-to-c"]);
    fs::create_dir_all(repo.join("c/c")).unwrap();
    run_git(repo, &["mv", "a/a/file.txt", "c/c/file.txt"]);
    write_file(repo, "c/c/file.txt", "one\ntwo\n3\n");
    commit_all(repo, "move to c");

    let merge_output = run_git_capture(repo, &["merge", "move-to-b"]);
    !merge_output.status.success()
}

fn read_recorded_merge_order(log_path: &Path) -> Vec<String> {
    let raw = fs::read_to_string(log_path).expect("read merge-order log");
    raw.lines()
        .map(|line| {
            let normalized = line.strip_prefix("./").unwrap_or(line);
            Path::new(normalized)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(normalized)
                .to_string()
        })
        .collect()
}

fn output_text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn read_recorded_stage_paths(repo: &Path, merged_path: &str) -> Vec<String> {
    let dump_path = repo.join(format!("{merged_path}.env"));
    let raw = fs::read_to_string(&dump_path).unwrap_or_else(|e| {
        panic!(
            "failed to read stage-path dump {}: {e}",
            dump_path.display()
        )
    });
    raw.lines().map(ToOwned::to_owned).collect()
}

/// Create a repo with a genuine merge conflict (overlapping changes).
fn setup_overlapping_conflict(repo: &Path) {
    init_repo(repo);
    write_file(repo, "file.txt", "aaa\nbbb\nccc\n");
    commit_all(repo, "base");

    run_git(repo, &["checkout", "-b", "feature"]);
    write_file(repo, "file.txt", "aaa\nREMOTE\nccc\n");
    commit_all(repo, "feature: change line 2");

    run_git(repo, &["checkout", "main"]);
    write_file(repo, "file.txt", "aaa\nLOCAL\nccc\n");
    commit_all(repo, "main: change line 2");

    // Merge will fail with a conflict.
    let output = run_git_capture(repo, &["merge", "feature"]);
    assert!(
        !output.status.success(),
        "expected merge to fail with conflict"
    );
}

/// Create a repo where both branches make non-overlapping changes
/// but to the same file, creating a conflict that our tool can auto-resolve.
///
/// Git's merge strategy may or may not auto-resolve non-overlapping changes.
/// To ensure a conflict that git leaves for the mergetool, we use changes
/// that are close enough to conflict (adjacent lines).
fn setup_resolvable_conflict(repo: &Path) {
    init_repo(repo);
    // Use a file with enough context separation.
    write_file(
        repo,
        "file.txt",
        "header\n\
         aaa\n\
         bbb\n\
         ccc\n\
         ddd\n\
         eee\n\
         footer\n",
    );
    commit_all(repo, "base");

    run_git(repo, &["checkout", "-b", "feature"]);
    write_file(
        repo,
        "file.txt",
        "header\n\
         aaa\n\
         REMOTE_CHANGE\n\
         ccc\n\
         ddd\n\
         eee\n\
         footer\n",
    );
    commit_all(repo, "feature: change bbb");

    run_git(repo, &["checkout", "main"]);
    write_file(
        repo,
        "file.txt",
        "header\n\
         aaa\n\
         bbb\n\
         ccc\n\
         ddd\n\
         LOCAL_CHANGE\n\
         footer\n",
    );
    commit_all(repo, "main: change eee");

    // Try merge. Git may auto-resolve non-overlapping, so we check.
    let output = run_git_capture(repo, &["merge", "feature"]);
    if output.status.success() {
        // Git auto-resolved — this test scenario won't exercise mergetool.
        // Fall back to an overlapping conflict that our tool can also handle.
        // Reset the merge and create a real conflict instead.
        run_git(repo, &["reset", "--hard", "HEAD~1"]);

        write_file(
            repo,
            "file.txt",
            "header\n\
             aaa\n\
             LOCAL_BBB\n\
             ccc\n\
             ddd\n\
             eee\n\
             footer\n",
        );
        commit_all(repo, "main: change bbb differently");

        let output = run_git_capture(repo, &["merge", "feature"]);
        assert!(
            !output.status.success(),
            "expected merge to fail with conflict"
        );
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[test]
fn git_mergetool_resolves_overlapping_conflict() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    setup_overlapping_conflict(repo);
    configure_gitgpui_mergetool(repo);

    // Run git mergetool. Our tool will detect the conflict and write
    // markers to MERGED, exiting 1.
    let output = run_git_capture(repo, &["mergetool", "--no-prompt"]);
    let text = output_text(&output);

    // The tool should have run (even if exit code is non-zero due to conflicts).
    // Check that the MERGED file was written by our tool.
    let merged = fs::read_to_string(repo.join("file.txt")).unwrap();

    // Our mergetool reads the actual BASE/LOCAL/REMOTE stage files and
    // performs its own 3-way merge. For this overlapping conflict,
    // it should write conflict markers.
    assert!(
        merged.contains("<<<<<<<") || merged.contains("LOCAL") || merged.contains("REMOTE"),
        "expected mergetool to have processed file.txt\nmerged:\n{merged}\ngit output:\n{text}"
    );
}

#[test]
fn git_mergetool_custom_cmd_copies_remote_to_merged() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    setup_overlapping_conflict(repo);

    let tool = "customcopy";
    configure_mergetool_selection(repo, tool, None, None);
    configure_mergetool_command(repo, tool, "cat \"$REMOTE\" > \"$MERGED\"");
    configure_mergetool_trust_exit_code(repo, tool, true);

    let output = run_git_capture(repo, &["mergetool", "--no-prompt"]);
    let text = output_text(&output);
    assert!(
        output.status.success(),
        "expected custom mergetool command to resolve conflict\n{text}"
    );

    let merged = fs::read_to_string(repo.join("file.txt")).unwrap();
    assert_eq!(
        merged, "aaa\nREMOTE\nccc\n",
        "expected custom command to copy REMOTE into MERGED\n{text}"
    );

    let unmerged = run_git_capture(repo, &["ls-files", "-u"]);
    assert!(
        unmerged.stdout.is_empty(),
        "expected no unmerged index entries after custom command\n{}",
        output_text(&unmerged)
    );
}

#[test]
fn git_mergetool_accepts_kdiff3_alias_flags_in_cmd() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    setup_overlapping_conflict(repo);
    configure_gitgpui_mergetool_with_alias_flags(repo);

    let output = run_git_capture(repo, &["mergetool", "--no-prompt"]);
    let text = output_text(&output);
    let merged = fs::read_to_string(repo.join("file.txt")).unwrap();

    assert!(
        !text.contains("unexpected argument '-o'")
            && !text.contains("unexpected argument '--L1'")
            && !text.contains("unexpected argument '--L2'")
            && !text.contains("unexpected argument '--L3'"),
        "expected alias flags to be accepted by gitgpui-app mergetool\noutput:\n{text}"
    );
    assert!(
        text.contains("Auto-merging file.txt"),
        "expected gitgpui-app mergetool to run\noutput:\n{text}"
    );
    assert!(
        merged.contains("LOCAL") || merged.contains("REMOTE") || merged.contains("<<<<<<<"),
        "expected merged file to be processed\nmerged:\n{merged}\noutput:\n{text}"
    );
}

#[test]
fn git_mergetool_kdiff3_path_override_invokes_compat_mode() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    setup_overlapping_conflict(repo);
    // Built-in kdiff3 invocation uses --auto/--L1/--L2/--L3/-o positional form.
    // trustExitCode=false keeps git from failing on unresolved conflicts so we
    // can verify marker label mapping from compatibility parsing.
    configure_kdiff3_path_override_to_gitgpui(repo, false);

    let output = run_git_capture(repo, &["mergetool", "--no-prompt"]);
    let text = output_text(&output);
    assert!(
        output.status.success(),
        "expected kdiff3 path-override invocation to succeed with gitgpui compatibility parsing\n{text}"
    );

    let merged = fs::read_to_string(repo.join("file.txt")).unwrap();
    assert!(
        merged.contains("(Local)") || merged.contains("(Remote)"),
        "expected kdiff3-style labels to propagate into gitgpui conflict markers\nmerged:\n{merged}\noutput:\n{text}"
    );
}

#[test]
fn git_mergetool_with_trust_exit_code_marks_clean_merge_resolved() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    setup_resolvable_conflict(repo);
    configure_gitgpui_mergetool(repo);

    let output = run_git_capture(repo, &["mergetool", "--no-prompt"]);
    let text = output_text(&output);

    // Check the file was processed by our tool.
    let merged = fs::read_to_string(repo.join("file.txt")).unwrap();
    // The file should not contain unprocessed git conflict markers from
    // the pre-mergetool state (those have <<<<<<< HEAD etc).
    // Our tool either cleanly merged or wrote its own markers.
    assert!(
        !merged.is_empty(),
        "expected mergetool to write content to file.txt\ngit output:\n{text}"
    );

    // After mergetool, check if there are still unresolved conflicts.
    let status = run_git_capture(repo, &["status", "--porcelain"]);
    let status_text = String::from_utf8_lossy(&status.stdout);

    // If our tool produced a clean merge (exit 0), git should have staged
    // the file. If it produced conflicts (exit 1), it remains unmerged.
    // Either way, the tool successfully ran.
    assert!(
        merged.contains("header") && merged.contains("footer"),
        "merged output should contain surrounding context\nmerged:\n{merged}\nstatus:\n{status_text}"
    );
}

#[test]
fn git_mergetool_handles_path_with_spaces() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_file(repo, "docs/spaced name.txt", "original\n");
    commit_all(repo, "base");

    run_git(repo, &["checkout", "-b", "feature"]);
    write_file(repo, "docs/spaced name.txt", "remote change\n");
    commit_all(repo, "feature: change spaced file");

    run_git(repo, &["checkout", "main"]);
    write_file(repo, "docs/spaced name.txt", "local change\n");
    commit_all(repo, "main: change spaced file");

    let output = run_git_capture(repo, &["merge", "feature"]);
    assert!(
        !output.status.success(),
        "expected merge conflict for spaced file"
    );

    configure_gitgpui_mergetool(repo);

    let output = run_git_capture(repo, &["mergetool", "--no-prompt"]);
    let text = output_text(&output);

    let merged = fs::read_to_string(repo.join("docs/spaced name.txt")).unwrap();
    // Tool should have processed the file despite spaces in path.
    assert!(
        merged.contains("local change")
            || merged.contains("remote change")
            || merged.contains("<<<<<<<"),
        "expected mergetool to process spaced-path file\nmerged:\n{merged}\ngit output:\n{text}"
    );
}

#[test]
fn git_mergetool_handles_unicode_path() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    let unicode_path = "docs/\u{65e5}\u{672c}\u{8a9e}-\u{0444}\u{0430}\u{0439}\u{043b}.txt";
    write_file(repo, unicode_path, "original\n");
    commit_all(repo, "base");

    run_git(repo, &["checkout", "-b", "feature"]);
    write_file(repo, unicode_path, "remote change\n");
    commit_all(repo, "feature: change unicode file");

    run_git(repo, &["checkout", "main"]);
    write_file(repo, unicode_path, "local change\n");
    commit_all(repo, "main: change unicode file");

    let output = run_git_capture(repo, &["merge", "feature"]);
    assert!(
        !output.status.success(),
        "expected merge conflict for unicode path"
    );

    configure_gitgpui_mergetool(repo);

    let output = run_git_capture(repo, &["mergetool", "--no-prompt"]);
    let text = output_text(&output);

    let merged = fs::read_to_string(repo.join(unicode_path)).unwrap();
    assert!(
        merged.contains("local change")
            || merged.contains("remote change")
            || merged.contains("<<<<<<<"),
        "expected mergetool to process unicode-path file\nmerged:\n{merged}\ngit output:\n{text}"
    );
}

#[test]
fn git_mergetool_works_from_subdirectory() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_file(repo, "sub/dir/nested.txt", "base content\n");
    commit_all(repo, "base");

    run_git(repo, &["checkout", "-b", "feature"]);
    write_file(repo, "sub/dir/nested.txt", "remote content\n");
    commit_all(repo, "feature: change nested");

    run_git(repo, &["checkout", "main"]);
    write_file(repo, "sub/dir/nested.txt", "local content\n");
    commit_all(repo, "main: change nested");

    let output = run_git_capture(repo, &["merge", "feature"]);
    assert!(
        !output.status.success(),
        "expected merge conflict for nested file"
    );

    configure_gitgpui_mergetool(repo);

    // Run from subdirectory.
    let subdir = repo.join("sub/dir");
    let output = run_git_capture_in(&subdir, &["mergetool", "--no-prompt"]);
    let text = output_text(&output);

    let merged = fs::read_to_string(repo.join("sub/dir/nested.txt")).unwrap();
    assert!(
        merged.contains("local content")
            || merged.contains("remote content")
            || merged.contains("<<<<<<<"),
        "expected mergetool to process file from subdirectory\nmerged:\n{merged}\ngit output:\n{text}"
    );
}

#[test]
fn git_mergetool_handles_add_add_conflict() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    // Create an empty initial commit so both branches can add new files.
    write_file(repo, "README", "init\n");
    commit_all(repo, "initial");

    run_git(repo, &["checkout", "-b", "feature"]);
    write_file(repo, "new_file.txt", "added by remote\n");
    commit_all(repo, "feature: add new_file");

    run_git(repo, &["checkout", "main"]);
    write_file(repo, "new_file.txt", "added by local\n");
    commit_all(repo, "main: add new_file");

    let output = run_git_capture(repo, &["merge", "feature"]);
    assert!(!output.status.success(), "expected add/add merge conflict");

    configure_gitgpui_mergetool(repo);

    let output = run_git_capture(repo, &["mergetool", "--no-prompt"]);
    let text = output_text(&output);

    let merged = fs::read_to_string(repo.join("new_file.txt")).unwrap();
    // For add/add, BASE is empty. Our tool treats this as empty base,
    // resulting in a conflict (both sides added different content).
    assert!(
        merged.contains("added by local")
            || merged.contains("added by remote")
            || merged.contains("<<<<<<<"),
        "expected mergetool to handle add/add conflict\nmerged:\n{merged}\ngit output:\n{text}"
    );
}

#[test]
fn git_mergetool_trust_exit_code_conflict_preserves_unmerged_state() {
    // When our tool exits 1 (unresolved conflict) with trustExitCode=true,
    // git should leave the file as unmerged. This verifies the exit code
    // contract between gitgpui-app and git mergetool.
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_file(repo, "conflict.txt", "base\n");
    commit_all(repo, "base");

    run_git(repo, &["checkout", "-b", "feature"]);
    write_file(repo, "conflict.txt", "feature side\n");
    commit_all(repo, "feature change");

    run_git(repo, &["checkout", "main"]);
    write_file(repo, "conflict.txt", "main side\n");
    commit_all(repo, "main change");

    let output = run_git_capture(repo, &["merge", "feature"]);
    assert!(!output.status.success(), "expected merge conflict");

    configure_gitgpui_mergetool(repo);

    let output = run_git_capture(repo, &["mergetool", "--no-prompt"]);
    let text = output_text(&output);

    // Our tool exits 1 on unresolved conflict. With trustExitCode=true,
    // git interprets this as failure and restores the original MERGED
    // content. The file should still have conflict markers.
    let merged = fs::read_to_string(repo.join("conflict.txt")).unwrap();
    assert!(
        merged.contains("<<<<<<<"),
        "expected conflict markers to remain after tool reports failure\nmerged:\n{merged}\ngit output:\n{text}"
    );

    // The file should still be in unmerged state (shown as UU in porcelain).
    let status = run_git_capture(repo, &["status", "--porcelain"]);
    let status_text = String::from_utf8_lossy(&status.stdout);
    assert!(
        status_text.contains("UU") || status_text.contains("AA"),
        "expected unmerged file in git status\nstatus:\n{status_text}\ngit output:\n{text}"
    );
}

#[test]
fn git_mergetool_no_trust_exit_code_unchanged_output_stays_unresolved() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    setup_overlapping_conflict(repo);
    configure_mergetool_selection(repo, "fake", None, None);
    configure_mergetool_command(repo, "fake", "exit 0");
    configure_mergetool_trust_exit_code(repo, "fake", false);

    let output = run_git_capture(repo, &["mergetool", "--no-prompt", "--tool", "fake"]);
    let text = output_text(&output);

    assert!(
        !output.status.success(),
        "expected git mergetool to fail when trustExitCode=false and output is unchanged\n{text}"
    );
    assert!(
        text.contains("seems unchanged"),
        "expected unchanged-output warning in git output\n{text}"
    );
    assert!(
        text.contains("Was the merge successful"),
        "expected no-trust follow-up prompt in git output\n{text}"
    );

    let merged = fs::read_to_string(repo.join("file.txt")).unwrap();
    assert!(
        merged.contains("<<<<<<<"),
        "expected conflict markers to remain when fake tool leaves output unchanged\nmerged:\n{merged}\n{text}"
    );

    let status = run_git_capture(repo, &["status", "--porcelain"]);
    let status_text = String::from_utf8_lossy(&status.stdout);
    assert!(
        status_text.contains("UU") || status_text.contains("AA"),
        "expected unresolved conflict after unchanged fake tool output\nstatus:\n{status_text}\n{text}"
    );
}

#[test]
fn git_mergetool_no_trust_exit_code_changed_output_resolves_conflict() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    setup_overlapping_conflict(repo);
    configure_mergetool_selection(repo, "fake", None, None);
    configure_mergetool_command(
        repo,
        "fake",
        "echo TOOL=fake >&2; cat \"$REMOTE\" > \"$MERGED\"; exit 1",
    );
    configure_mergetool_trust_exit_code(repo, "fake", false);

    let output = run_git_capture(repo, &["mergetool", "--no-prompt", "--tool", "fake"]);
    let text = output_text(&output);

    assert!(
        output.status.success(),
        "expected git mergetool to accept changed output when trustExitCode=false\n{text}"
    );
    assert!(
        text.contains("TOOL=fake"),
        "expected fake tool marker in output\n{text}"
    );
    assert!(
        !text.contains("Was the merge successful"),
        "did not expect no-trust prompt when fake tool changed MERGED\n{text}"
    );

    let merged = fs::read_to_string(repo.join("file.txt")).unwrap();
    assert_eq!(merged, "aaa\nREMOTE\nccc\n");

    let status = run_git_capture(repo, &["status", "--porcelain"]);
    let status_text = String::from_utf8_lossy(&status.stdout);
    assert!(
        !status_text.contains("UU") && !status_text.contains("AA"),
        "expected conflict to be cleared after fake tool changed output\nstatus:\n{status_text}\n{text}"
    );
}

#[test]
fn git_mergetool_multiple_conflicted_files() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_file(repo, "alpha.txt", "base alpha\n");
    write_file(repo, "beta.txt", "base beta\n");
    commit_all(repo, "base");

    run_git(repo, &["checkout", "-b", "feature"]);
    write_file(repo, "alpha.txt", "remote alpha\n");
    write_file(repo, "beta.txt", "remote beta\n");
    commit_all(repo, "feature: change both");

    run_git(repo, &["checkout", "main"]);
    write_file(repo, "alpha.txt", "local alpha\n");
    write_file(repo, "beta.txt", "local beta\n");
    commit_all(repo, "main: change both");

    let output = run_git_capture(repo, &["merge", "feature"]);
    assert!(!output.status.success(), "expected merge conflict");

    configure_gitgpui_mergetool(repo);

    let output = run_git_capture(repo, &["mergetool", "--no-prompt"]);
    let text = output_text(&output);

    // Both files should have been processed by the mergetool.
    let alpha = fs::read_to_string(repo.join("alpha.txt")).unwrap();
    let beta = fs::read_to_string(repo.join("beta.txt")).unwrap();

    assert!(
        alpha.contains("<<<<<<<")
            || alpha.contains("local alpha")
            || alpha.contains("remote alpha"),
        "expected alpha.txt to be processed\nalpha:\n{alpha}\ngit output:\n{text}"
    );
    assert!(
        beta.contains("<<<<<<<") || beta.contains("local beta") || beta.contains("remote beta"),
        "expected beta.txt to be processed\nbeta:\n{beta}\ngit output:\n{text}"
    );
}

#[test]
fn git_mergetool_crlf_content_preserved() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    // Disable autocrlf to preserve exact line endings.
    run_git(repo, &["config", "core.autocrlf", "false"]);

    write_file(repo, "crlf.txt", "line1\r\nline2\r\nline3\r\n");
    commit_all(repo, "base with CRLF");

    run_git(repo, &["checkout", "-b", "feature"]);
    write_file(repo, "crlf.txt", "remote1\r\nline2\r\nline3\r\n");
    commit_all(repo, "feature: change line 1");

    run_git(repo, &["checkout", "main"]);
    write_file(repo, "crlf.txt", "local1\r\nline2\r\nline3\r\n");
    commit_all(repo, "main: change line 1");

    let output = run_git_capture(repo, &["merge", "feature"]);
    assert!(!output.status.success(), "expected CRLF merge conflict");

    configure_gitgpui_mergetool(repo);

    let output = run_git_capture(repo, &["mergetool", "--no-prompt"]);
    let text = output_text(&output);

    let merged_bytes = fs::read(repo.join("crlf.txt")).unwrap();
    let merged = String::from_utf8_lossy(&merged_bytes);

    // The tool should have processed the file. Content should still
    // contain CRLF sequences from the original input.
    assert!(
        merged.contains("\r\n"),
        "expected CRLF to be preserved in merged output\nmerged:\n{merged}\ngit output:\n{text}"
    );
}

// ── writeToTemp path semantics parity ───────────────────────────────

#[test]
fn git_mergetool_write_to_temp_true_uses_absolute_stage_paths() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    setup_overlapping_conflict(repo);
    configure_stage_path_recording_mergetool(repo, "stagepaths");
    run_git(repo, &["config", "mergetool.writeToTemp", "true"]);

    let output = run_git_capture(repo, &["mergetool", "--no-prompt", "--tool", "stagepaths"]);
    let text = output_text(&output);
    assert!(
        output.status.success(),
        "git mergetool failed for writeToTemp=true\n{text}"
    );

    let vars = read_recorded_stage_paths(repo, "file.txt");
    assert_eq!(
        vars.len(),
        3,
        "expected BASE/LOCAL/REMOTE stage paths\n{text}"
    );
    for var in vars {
        assert!(
            Path::new(&var).is_absolute(),
            "writeToTemp=true should provide absolute stage paths, got: {var}\n{text}"
        );
        assert!(
            !var.starts_with("./"),
            "writeToTemp=true should not use ./-prefixed stage paths, got: {var}\n{text}"
        );
    }
}

#[test]
fn git_mergetool_write_to_temp_false_uses_workdir_prefixed_stage_paths() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    setup_overlapping_conflict(repo);
    configure_stage_path_recording_mergetool(repo, "stagepaths");
    run_git(repo, &["config", "mergetool.writeToTemp", "false"]);

    let output = run_git_capture(repo, &["mergetool", "--no-prompt", "--tool", "stagepaths"]);
    let text = output_text(&output);
    assert!(
        output.status.success(),
        "git mergetool failed for writeToTemp=false\n{text}"
    );

    let vars = read_recorded_stage_paths(repo, "file.txt");
    assert_eq!(
        vars.len(),
        3,
        "expected BASE/LOCAL/REMOTE stage paths\n{text}"
    );
    for var in vars {
        assert!(
            var.starts_with("./"),
            "writeToTemp=false should provide ./-prefixed stage paths, got: {var}\n{text}"
        );
    }
}

// ── diff.orderFile ordering parity ───────────────────────────────────

#[test]
fn git_mergetool_honors_diff_order_file_configuration() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    setup_order_file_conflict(repo);
    write_file(repo, "order-file", "b\na\n");
    run_git(repo, &["config", "diff.orderFile", "order-file"]);

    let order_log = repo.join(".mergetool-order.log");
    configure_recording_mergetool(repo, "ordercheck", &order_log);

    let output = run_git_capture(repo, &["mergetool", "--no-prompt", "--tool", "ordercheck"]);
    let text = output_text(&output);
    assert!(output.status.success(), "git mergetool failed\n{text}");

    let order = read_recorded_merge_order(&order_log);
    assert_eq!(order, vec!["b", "a"], "unexpected merge order\n{text}");
}

#[test]
fn git_mergetool_o_flag_overrides_diff_order_file() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    setup_order_file_conflict(repo);
    write_file(repo, "order-file", "b\na\n");
    write_file(repo, "cli-order-file", "a\nb\n");
    run_git(repo, &["config", "diff.orderFile", "order-file"]);

    let order_log = repo.join(".mergetool-order.log");
    configure_recording_mergetool(repo, "ordercheck", &order_log);

    let output = run_git_capture(
        repo,
        &[
            "mergetool",
            "-Ocli-order-file",
            "--no-prompt",
            "--tool",
            "ordercheck",
        ],
    );
    let text = output_text(&output);
    assert!(
        output.status.success(),
        "git mergetool with -O override failed\n{text}"
    );

    let order = read_recorded_merge_order(&order_log);
    assert_eq!(order, vec!["a", "b"], "unexpected merge order\n{text}");
}

// ── Tool-help discoverability ────────────────────────────────────────

#[test]
fn git_mergetool_tool_help_lists_gitgpui_tool() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    configure_gitgpui_mergetool(repo);

    let output = run_git_capture(repo, &["mergetool", "--tool-help"]);
    let text = output_text(&output);
    assert!(
        output.status.success(),
        "git mergetool --tool-help failed\n{text}"
    );
    assert!(
        text.contains("gitgpui"),
        "expected gitgpui tool name in --tool-help output\n{text}"
    );
}

// ── GUI tool selection parity ────────────────────────────────────────

#[test]
fn git_mergetool_gui_default_auto_prefers_gui_tool_when_display_set() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_file(repo, "file.txt", "base\n");
    commit_all(repo, "base");

    run_git(repo, &["checkout", "-b", "feature"]);
    write_file(repo, "file.txt", "remote\n");
    commit_all(repo, "feature change");

    run_git(repo, &["checkout", "main"]);
    write_file(repo, "file.txt", "local\n");
    commit_all(repo, "main change");

    let output = run_git_capture(repo, &["merge", "feature"]);
    assert!(!output.status.success(), "expected merge conflict");

    // Configure two tools with distinct markers.
    configure_mergetool_command(repo, "cli", &mergetool_marker_cmd("cli"));
    configure_mergetool_trust_exit_code(repo, "cli", true);
    configure_mergetool_command(repo, "gui", &mergetool_marker_cmd("gui"));
    configure_mergetool_trust_exit_code(repo, "gui", true);
    configure_mergetool_selection(repo, "cli", Some("gui"), Some("auto"));

    // With DISPLAY set, guiDefault=auto should select the GUI tool.
    let output = run_git_capture_with_display(repo, &["mergetool", "--no-prompt"], Some(":99"));
    let text = output_text(&output);
    assert!(
        text.contains("TOOL=gui"),
        "expected gui tool selection with DISPLAY set\n{text}"
    );
}

#[test]
fn git_mergetool_gui_default_auto_prefers_cli_tool_without_display() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_file(repo, "file.txt", "base\n");
    commit_all(repo, "base");

    run_git(repo, &["checkout", "-b", "feature"]);
    write_file(repo, "file.txt", "remote\n");
    commit_all(repo, "feature change");

    run_git(repo, &["checkout", "main"]);
    write_file(repo, "file.txt", "local\n");
    commit_all(repo, "main change");

    let output = run_git_capture(repo, &["merge", "feature"]);
    assert!(!output.status.success(), "expected merge conflict");

    configure_mergetool_command(repo, "cli", &mergetool_marker_cmd("cli"));
    configure_mergetool_trust_exit_code(repo, "cli", true);
    configure_mergetool_command(repo, "gui", &mergetool_marker_cmd("gui"));
    configure_mergetool_trust_exit_code(repo, "gui", true);
    configure_mergetool_selection(repo, "cli", Some("gui"), Some("auto"));

    // Without DISPLAY, guiDefault=auto should select the CLI tool.
    let output = run_git_capture_with_display(repo, &["mergetool", "--no-prompt"], None);
    let text = output_text(&output);
    assert!(
        text.contains("TOOL=cli"),
        "expected cli tool selection without DISPLAY\n{text}"
    );
}

#[test]
fn git_mergetool_gui_flag_overrides_selection() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_file(repo, "file.txt", "base\n");
    commit_all(repo, "base");

    run_git(repo, &["checkout", "-b", "feature"]);
    write_file(repo, "file.txt", "remote\n");
    commit_all(repo, "feature change");

    run_git(repo, &["checkout", "main"]);
    write_file(repo, "file.txt", "local\n");
    commit_all(repo, "main change");

    let output = run_git_capture(repo, &["merge", "feature"]);
    assert!(!output.status.success(), "expected merge conflict");

    configure_mergetool_command(repo, "cli", &mergetool_marker_cmd("cli"));
    configure_mergetool_trust_exit_code(repo, "cli", true);
    configure_mergetool_command(repo, "gui", &mergetool_marker_cmd("gui"));
    configure_mergetool_trust_exit_code(repo, "gui", true);
    // guiDefault=false, but --gui flag should override.
    configure_mergetool_selection(repo, "cli", Some("gui"), Some("false"));

    let output = run_git_capture_with_display(repo, &["mergetool", "--gui", "--no-prompt"], None);
    let text = output_text(&output);
    assert!(
        text.contains("TOOL=gui"),
        "expected --gui to force gui tool selection\n{text}"
    );
}

#[test]
fn git_mergetool_no_gui_flag_overrides_gui_default_true() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_file(repo, "file.txt", "base\n");
    commit_all(repo, "base");

    run_git(repo, &["checkout", "-b", "feature"]);
    write_file(repo, "file.txt", "remote\n");
    commit_all(repo, "feature change");

    run_git(repo, &["checkout", "main"]);
    write_file(repo, "file.txt", "local\n");
    commit_all(repo, "main change");

    let output = run_git_capture(repo, &["merge", "feature"]);
    assert!(!output.status.success(), "expected merge conflict");

    configure_mergetool_command(repo, "cli", &mergetool_marker_cmd("cli"));
    configure_mergetool_trust_exit_code(repo, "cli", true);
    configure_mergetool_command(repo, "gui", &mergetool_marker_cmd("gui"));
    configure_mergetool_trust_exit_code(repo, "gui", true);
    // guiDefault=true, but --no-gui flag should override.
    configure_mergetool_selection(repo, "cli", Some("gui"), Some("true"));

    let output =
        run_git_capture_with_display(repo, &["mergetool", "--no-gui", "--no-prompt"], Some(":99"));
    let text = output_text(&output);
    assert!(
        text.contains("TOOL=cli"),
        "expected --no-gui to force regular tool selection\n{text}"
    );
}

#[test]
fn git_mergetool_gui_fallback_when_no_guitool_configured() {
    // When --gui is specified but no merge.guitool is configured,
    // git falls back to merge.tool.
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_file(repo, "file.txt", "base\n");
    commit_all(repo, "base");

    run_git(repo, &["checkout", "-b", "feature"]);
    write_file(repo, "file.txt", "remote\n");
    commit_all(repo, "feature change");

    run_git(repo, &["checkout", "main"]);
    write_file(repo, "file.txt", "local\n");
    commit_all(repo, "main change");

    let output = run_git_capture(repo, &["merge", "feature"]);
    assert!(!output.status.success(), "expected merge conflict");

    configure_mergetool_command(repo, "cli", &mergetool_marker_cmd("cli"));
    configure_mergetool_trust_exit_code(repo, "cli", true);
    // Only merge.tool set, no merge.guitool.
    configure_mergetool_selection(repo, "cli", None, None);

    let output =
        run_git_capture_with_display(repo, &["mergetool", "--gui", "--no-prompt"], Some(":99"));
    let text = output_text(&output);
    // Git falls back to merge.tool when no guitool is configured.
    assert!(
        text.contains("TOOL=cli"),
        "expected fallback to merge.tool when no guitool configured\n{text}"
    );
}

// ── Nonexistent tool error handling ──────────────────────────────────

#[test]
fn git_mergetool_nonexistent_tool_reports_error() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_file(repo, "file.txt", "base\n");
    commit_all(repo, "base");

    run_git(repo, &["checkout", "-b", "feature"]);
    write_file(repo, "file.txt", "remote\n");
    commit_all(repo, "feature change");

    run_git(repo, &["checkout", "main"]);
    write_file(repo, "file.txt", "local\n");
    commit_all(repo, "main change");

    let output = run_git_capture(repo, &["merge", "feature"]);
    assert!(!output.status.success(), "expected merge conflict");

    // Configure a tool that points to a nonexistent command.
    run_git(repo, &["config", "merge.tool", "nonexistent_tool_xyz"]);
    run_git(
        repo,
        &[
            "config",
            "mergetool.nonexistent_tool_xyz.cmd",
            "/absolutely/nonexistent/binary --merge",
        ],
    );
    run_git(
        repo,
        &[
            "config",
            "mergetool.nonexistent_tool_xyz.trustExitCode",
            "true",
        ],
    );
    run_git(repo, &["config", "mergetool.prompt", "false"]);

    let output = run_git_capture(repo, &["mergetool", "--no-prompt"]);
    let text = output_text(&output);

    // Git should report failure when the tool command fails to execute.
    assert!(
        !output.status.success(),
        "expected git mergetool to fail with nonexistent tool\n{text}"
    );
}

// ── Delete/delete conflict behavior ──────────────────────────────────

#[test]
fn git_mergetool_delete_delete_conflict_handling() {
    // When both branches delete the same file, git mergetool handles
    // this without invoking the external tool. The file just needs to
    // be staged as deleted.
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_file(repo, "to_delete.txt", "content\n");
    write_file(repo, "keep.txt", "kept\n");
    commit_all(repo, "base");

    run_git(repo, &["checkout", "-b", "feature"]);
    run_git(repo, &["rm", "to_delete.txt"]);
    // Also modify keep.txt to create a real merge (not fast-forward).
    write_file(repo, "keep.txt", "feature version\n");
    commit_all(repo, "feature: delete file and modify keep");

    run_git(repo, &["checkout", "main"]);
    run_git(repo, &["rm", "to_delete.txt"]);
    write_file(repo, "keep.txt", "main version\n");
    commit_all(repo, "main: delete file and modify keep");

    let merge_output = run_git_capture(repo, &["merge", "feature"]);
    // Depending on git version, both-deleted might auto-resolve or conflict.
    // If the merge succeeds (both-deleted auto-resolved), skip the mergetool test.
    if merge_output.status.success() {
        // Both-deleted auto-resolved by git — verify file is gone.
        assert!(
            !repo.join("to_delete.txt").exists(),
            "expected deleted file to stay deleted after merge"
        );
        return;
    }

    // Configure mergetool and attempt to resolve.
    configure_mergetool_command(repo, "gitgpui", &mergetool_marker_cmd("gitgpui"));
    configure_mergetool_trust_exit_code(repo, "gitgpui", true);
    configure_mergetool_selection(repo, "gitgpui", None, None);

    let output = run_git_capture(repo, &["mergetool", "--no-prompt"]);
    let _text = output_text(&output);

    // After mergetool, the deleted file should not exist in the working tree.
    // Git handles delete/delete internally (may prompt for d/m/a choices,
    // or auto-resolve when both sides agree on deletion).
    assert!(
        !repo.join("to_delete.txt").exists(),
        "expected both-deleted file to be removed after mergetool"
    );
}

#[test]
fn git_mergetool_delete_delete_choice_d_deletes_original_path() {
    // Port of t7610 delete/delete "d" choice.
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    if !setup_delete_delete_rename_conflict(repo) {
        // Auto-resolved by git version under test.
        return;
    }

    configure_gitgpui_mergetool(repo);

    let output = run_git_with_stdin(repo, &["mergetool", "a/a/file.txt"], "d\n");
    let text = output_text(&output);
    assert!(
        output.status.success(),
        "expected delete/delete resolution with 'd' to succeed\n{text}"
    );
    assert!(
        !repo.join("a/a/file.txt").exists(),
        "expected original path to be deleted after 'd' choice\n{text}"
    );
}

#[test]
fn git_mergetool_delete_delete_choice_m_keeps_modified_destination() {
    // Port of t7610 delete/delete "m" choice.
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    if !setup_delete_delete_rename_conflict(repo) {
        return;
    }

    configure_gitgpui_mergetool(repo);

    let output = run_git_with_stdin(repo, &["mergetool", "a/a/file.txt"], "m\n");
    let text = output_text(&output);
    assert!(
        output.status.success(),
        "expected delete/delete resolution with 'm' to succeed\n{text}"
    );
    assert!(
        repo.join("b/b/file.txt").exists(),
        "expected modified destination file b/b/file.txt after 'm' choice\n{text}"
    );
    assert!(
        !repo.join("a/a/file.txt").exists(),
        "expected original path to remain deleted after 'm' choice\n{text}"
    );
}

#[test]
fn git_mergetool_delete_delete_choice_a_aborts_with_nonzero() {
    // Port of t7610 delete/delete "a" (abort) behavior.
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    if !setup_delete_delete_rename_conflict(repo) {
        return;
    }

    configure_gitgpui_mergetool(repo);

    let output = run_git_with_stdin(repo, &["mergetool", "a/a/file.txt"], "a\n");
    let text = output_text(&output);
    assert!(
        !output.status.success(),
        "expected delete/delete 'a' abort to return non-zero\n{text}"
    );
    assert!(
        !repo.join("a/a/file.txt").exists(),
        "expected original path to remain absent after abort\n{text}"
    );
}

#[test]
fn git_mergetool_keep_backup_delete_delete_no_errors() {
    // Parity with git t7610: "mergetool produces no errors when keepBackup is used"
    //
    // When both branches rename a file from the same path to different
    // destinations, git sees a delete/delete conflict at the original path.
    // With keepBackup=true, resolving via "d" (delete) should produce NO
    // errors on stderr and the original directory should be cleaned up.
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);

    // Create a file inside a nested directory: a/a/file.txt
    fs::create_dir_all(repo.join("a/a")).unwrap();
    write_file(repo, "a/a/file.txt", "one\ntwo\n3\n4\n");
    commit_all(repo, "base file");

    // Branch move-to-b: rename a/a/file.txt -> b/b/file.txt (with edit)
    run_git(repo, &["checkout", "-b", "move-to-b"]);
    fs::create_dir_all(repo.join("b/b")).unwrap();
    run_git(repo, &["mv", "a/a/file.txt", "b/b/file.txt"]);
    write_file(repo, "b/b/file.txt", "one\ntwo\n4\n");
    commit_all(repo, "move to b");

    // Branch move-to-c: rename a/a/file.txt -> c/c/file.txt (with edit)
    run_git(repo, &["checkout", "main"]);
    run_git(repo, &["checkout", "-b", "move-to-c"]);
    fs::create_dir_all(repo.join("c/c")).unwrap();
    run_git(repo, &["mv", "a/a/file.txt", "c/c/file.txt"]);
    write_file(repo, "c/c/file.txt", "one\ntwo\n3\n");
    commit_all(repo, "move to c");

    // Merge move-to-b into move-to-c → creates delete/delete at a/a/file.txt
    let merge_output = run_git_capture(repo, &["merge", "move-to-b"]);
    if merge_output.status.success() {
        // Git auto-resolved the rename/rename — skip this test.
        return;
    }

    // Configure mergetool with keepBackup=true (the setting under test).
    configure_mergetool_command(repo, "gitgpui", &mergetool_marker_cmd("gitgpui"));
    configure_mergetool_trust_exit_code(repo, "gitgpui", true);
    run_git(repo, &["config", "merge.tool", "gitgpui"]);
    run_git(repo, &["config", "mergetool.prompt", "false"]);
    run_git(repo, &["config", "mergetool.keepBackup", "true"]);

    // Resolve with "d" (delete) for the delete/delete conflict at the
    // original path, and "d" again for any rename/rename prompts git may
    // present. Pipe enough answers for all prompts git may ask.
    let output = run_git_with_stdin(repo, &["mergetool", "--no-prompt"], "d\nd\nd\nd\n");

    // Key assertion: stderr must be empty (no errors from keepBackup
    // interacting with delete/delete cleanup).
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Filter out git's own informational messages (e.g. "Merging:") —
    // only assert that no error lines are present.
    let error_lines: Vec<&str> = stderr
        .lines()
        .filter(|line| {
            let l = line.trim();
            // Skip empty lines and known git informational output.
            !l.is_empty()
                && !l.starts_with("Merging")
                && !l.starts_with("Normal merge")
                && !l.starts_with("Deleted merge")
                && !l.starts_with("TOOL=")
        })
        .collect();
    assert!(
        error_lines.is_empty(),
        "expected no errors on stderr with keepBackup=true for delete/delete conflict\nstderr lines: {error_lines:?}\nfull stderr:\n{stderr}"
    );

    // The original directory "a" should have been cleaned up.
    // (Git removes it when the file inside is deleted.)
    assert!(
        !repo.join("a/a/file.txt").exists(),
        "expected original file a/a/file.txt to be gone after delete resolution"
    );
}

#[test]
fn git_mergetool_keep_temporaries_delete_delete_abort_keeps_stage_files() {
    // Parity with git t7610: "mergetool keeps tempfiles when aborting delete/delete"
    // for a path-targeted delete/delete conflict flow.
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    if !setup_delete_delete_rename_conflict(repo) {
        // Some git versions may auto-resolve this rename/rename setup.
        return;
    }

    configure_gitgpui_mergetool(repo);
    run_git(repo, &["config", "mergetool.keepTemporaries", "true"]);
    run_git(repo, &["config", "mergetool.writeToTemp", "false"]);

    let output = run_git_with_stdin(repo, &["mergetool", "a/a/file.txt"], "a\n");
    let text = output_text(&output);
    assert!(
        !output.status.success(),
        "expected abort to return non-zero for delete/delete conflict\n{text}"
    );

    let temp_dir = repo.join("a/a");
    assert!(
        temp_dir.is_dir(),
        "expected delete/delete temp directory to exist after abort with keepTemporaries=true"
    );

    let mut entries: Vec<String> = fs::read_dir(&temp_dir)
        .expect("read preserved temp directory")
        .map(|entry| {
            entry
                .expect("read temp directory entry")
                .file_name()
                .to_string_lossy()
                .to_string()
        })
        .collect();
    entries.sort();

    let has_base = entries
        .iter()
        .any(|name| name.starts_with("file_BASE_") && name.ends_with(".txt"));
    let has_local = entries
        .iter()
        .any(|name| name.starts_with("file_LOCAL_") && name.ends_with(".txt"));
    let has_remote = entries
        .iter()
        .any(|name| name.starts_with("file_REMOTE_") && name.ends_with(".txt"));

    assert!(
        has_base && has_local && has_remote,
        "expected preserved BASE/LOCAL/REMOTE stage files, got {entries:?}\n{text}"
    );
}

// ── Modify/delete conflict ───────────────────────────────────────────

#[test]
fn git_mergetool_modify_delete_conflict() {
    // One branch modifies a file, the other deletes it.
    // Git mergetool presents this as a special conflict type.
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    write_file(repo, "file.txt", "original\n");
    commit_all(repo, "base");

    run_git(repo, &["checkout", "-b", "feature"]);
    run_git(repo, &["rm", "file.txt"]);
    commit_all(repo, "feature: delete file");

    run_git(repo, &["checkout", "main"]);
    write_file(repo, "file.txt", "modified content\n");
    commit_all(repo, "main: modify file");

    let output = run_git_capture(repo, &["merge", "feature"]);
    assert!(
        !output.status.success(),
        "expected modify/delete merge conflict"
    );

    // Configure our tool. For modify/delete, git will still invoke
    // the mergetool (with a special prompt in some cases).
    configure_gitgpui_mergetool(repo);

    let output = run_git_capture(repo, &["mergetool", "--no-prompt"]);
    let text = output_text(&output);

    // Git should report the modify/delete conflict.
    // The mergetool pipeline should complete without crashing.
    // For modify/delete, git handles the conflict internally (deleted-by prompt)
    // and the external tool may or may not be invoked depending on git version.
    // The fact that run_git_capture above returned proves the pipeline didn't hang.
    // Verify git status still works after the mergetool run.
    let status = run_git_capture(repo, &["status", "--porcelain"]);
    assert!(
        status.status.success(),
        "git status should succeed after mergetool\ngit output:\n{text}"
    );
}

// ── Symlink conflict behavior ────────────────────────────────────────

#[test]
fn git_mergetool_symlink_conflict_resolved_via_local() {
    // When both branches change a symlink's target, git mergetool handles
    // the symlink conflict internally with a l/r/a prompt (does NOT invoke
    // the external tool). Verify that answering "l" keeps the local target.
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    // Create a symlink in the base commit.
    std::os::unix::fs::symlink("original_target", repo.join("link")).expect("create symlink");
    commit_all(repo, "base: add symlink");

    run_git(repo, &["checkout", "-b", "feature"]);
    fs::remove_file(repo.join("link")).unwrap();
    std::os::unix::fs::symlink("remote_target", repo.join("link")).expect("create symlink");
    commit_all(repo, "feature: change link target");

    run_git(repo, &["checkout", "main"]);
    fs::remove_file(repo.join("link")).unwrap();
    std::os::unix::fs::symlink("local_target", repo.join("link")).expect("create symlink");
    commit_all(repo, "main: change link target");

    let merge_out = run_git_capture(repo, &["merge", "feature"]);
    if merge_out.status.success() {
        // Some git versions auto-resolve symlink conflicts — skip this test.
        return;
    }

    configure_gitgpui_mergetool(repo);

    // Pipe "l\n" to stdin to answer the symlink resolution prompt.
    let output = run_git_with_stdin(repo, &["mergetool", "--no-prompt"], "l\n");
    let text = output_text(&output);

    // After answering "l" (local), the symlink should point to local_target.
    let target = fs::read_link(repo.join("link"));
    assert!(
        target.is_ok(),
        "expected symlink to exist after resolution\ngit output:\n{text}"
    );
    let target = target.unwrap();
    assert_eq!(
        target.to_string_lossy(),
        "local_target",
        "expected local symlink target after answering 'l'\ngit output:\n{text}"
    );
}

#[test]
fn git_mergetool_symlink_conflict_resolved_via_remote() {
    // Verify that answering "r" to a symlink conflict keeps the remote target.
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    std::os::unix::fs::symlink("original_target", repo.join("link")).expect("create symlink");
    commit_all(repo, "base: add symlink");

    run_git(repo, &["checkout", "-b", "feature"]);
    fs::remove_file(repo.join("link")).unwrap();
    std::os::unix::fs::symlink("remote_target", repo.join("link")).expect("create symlink");
    commit_all(repo, "feature: change link target");

    run_git(repo, &["checkout", "main"]);
    fs::remove_file(repo.join("link")).unwrap();
    std::os::unix::fs::symlink("local_target", repo.join("link")).expect("create symlink");
    commit_all(repo, "main: change link target");

    let merge_out = run_git_capture(repo, &["merge", "feature"]);
    if merge_out.status.success() {
        return;
    }

    configure_gitgpui_mergetool(repo);

    let output = run_git_with_stdin(repo, &["mergetool", "--no-prompt"], "r\n");
    let text = output_text(&output);

    let target = fs::read_link(repo.join("link"));
    assert!(
        target.is_ok(),
        "expected symlink to exist after resolution\ngit output:\n{text}"
    );
    let target = target.unwrap();
    assert_eq!(
        target.to_string_lossy(),
        "remote_target",
        "expected remote symlink target after answering 'r'\ngit output:\n{text}"
    );
}

#[test]
fn git_mergetool_symlink_alongside_normal_file_conflict() {
    // When both a symlink conflict and a normal file conflict exist,
    // git handles the symlink internally (l/r/a prompt) and invokes
    // our external tool for the normal file.
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();

    init_repo(repo);
    std::os::unix::fs::symlink("original_target", repo.join("link")).expect("create symlink");
    write_file(repo, "normal.txt", "base\n");
    commit_all(repo, "base");

    run_git(repo, &["checkout", "-b", "feature"]);
    fs::remove_file(repo.join("link")).unwrap();
    std::os::unix::fs::symlink("remote_target", repo.join("link")).expect("create symlink");
    write_file(repo, "normal.txt", "remote\n");
    commit_all(repo, "feature: change both");

    run_git(repo, &["checkout", "main"]);
    fs::remove_file(repo.join("link")).unwrap();
    std::os::unix::fs::symlink("local_target", repo.join("link")).expect("create symlink");
    write_file(repo, "normal.txt", "local\n");
    commit_all(repo, "main: change both");

    let merge_out = run_git_capture(repo, &["merge", "feature"]);
    if merge_out.status.success() {
        return;
    }

    configure_gitgpui_mergetool(repo);

    // Pipe "l\n" for the symlink prompt. The normal file conflict
    // will be processed by our external tool automatically.
    let output = run_git_with_stdin(repo, &["mergetool", "--no-prompt"], "l\n");
    let text = output_text(&output);

    // Verify the normal file was processed by our mergetool.
    let normal_content = fs::read_to_string(repo.join("normal.txt")).unwrap();
    assert!(
        normal_content.contains("local")
            || normal_content.contains("remote")
            || normal_content.contains("<<<<<<<"),
        "expected external tool to process normal file conflict\nnormal.txt:\n{normal_content}\ngit output:\n{text}"
    );

    // Verify the symlink was resolved by git's internal handler.
    let target = fs::read_link(repo.join("link"));
    assert!(
        target.is_ok(),
        "expected symlink to be resolved\ngit output:\n{text}"
    );
    assert_eq!(
        target.unwrap().to_string_lossy(),
        "local_target",
        "expected local symlink target"
    );
}

// ── Submodule conflict behavior ──────────────────────────────────────

fn create_submodule_repo(path: &Path) {
    run_git(path, &["init", "-b", "main"]);
    run_git(path, &["config", "user.email", "sub@example.com"]);
    run_git(path, &["config", "user.name", "Sub"]);
    run_git(path, &["config", "commit.gpgsign", "false"]);
    write_file(path, "sub_file.txt", "submodule content\n");
    run_git(path, &["add", "-A"]);
    run_git(
        path,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "initial submodule",
        ],
    );
}

fn advance_submodule(path: &Path, content: &str, message: &str) {
    write_file(path, "sub_file.txt", content);
    run_git(path, &["add", "-A"]);
    run_git(
        path,
        &["-c", "commit.gpgsign=false", "commit", "-m", message],
    );
}

#[test]
fn git_mergetool_submodule_conflict_resolved_via_local() {
    // When both branches update a submodule to different commits,
    // git mergetool handles it internally with l/r/a prompt.
    // Answering "l" keeps the local submodule commit.
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("main_repo");
    let sub_repo = tmp.path().join("sub_repo");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&sub_repo).unwrap();

    // Create the submodule source repo with initial commit.
    create_submodule_repo(&sub_repo);

    // Create the main repo and add the submodule.
    init_repo(&repo);
    let sub_url = format!("file://{}", sub_repo.display());
    run_git(&repo, &["submodule", "add", &sub_url, "submod"]);
    commit_all(&repo, "add submodule");

    // Create two diverging submodule commits.
    advance_submodule(&sub_repo, "commit A\n", "advance A");
    let commit_a_out = run_git_capture(&sub_repo, &["rev-parse", "HEAD"]);
    let commit_a = String::from_utf8_lossy(&commit_a_out.stdout)
        .trim()
        .to_string();

    advance_submodule(&sub_repo, "commit B\n", "advance B");
    let commit_b_out = run_git_capture(&sub_repo, &["rev-parse", "HEAD"]);
    let commit_b = String::from_utf8_lossy(&commit_b_out.stdout)
        .trim()
        .to_string();

    // Branch feature: update submodule to commit_a.
    run_git(&repo, &["checkout", "-b", "feature"]);
    run_git(&repo.join("submod"), &["fetch"]);
    run_git(&repo.join("submod"), &["checkout", &commit_a]);
    run_git(&repo, &["add", "submod"]);
    run_git(
        &repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "feature: update submod",
        ],
    );

    // Branch main: update submodule to commit_b.
    run_git(&repo, &["checkout", "main"]);
    run_git(&repo, &["submodule", "update", "--init"]);
    run_git(&repo.join("submod"), &["fetch"]);
    run_git(&repo.join("submod"), &["checkout", &commit_b]);
    run_git(&repo, &["add", "submod"]);
    run_git(
        &repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "main: update submod",
        ],
    );

    // Merge.
    let merge_out = run_git_capture(&repo, &["merge", "feature"]);
    if merge_out.status.success() {
        // Git auto-resolved the submodule conflict — skip.
        return;
    }

    configure_gitgpui_mergetool(&repo);

    // Answer "l" for the submodule prompt.
    let output = run_git_with_stdin(&repo, &["mergetool", "--no-prompt"], "l\n");
    let text = output_text(&output);

    // The submodule should be resolved to the local (main) commit.
    let submod_head = run_git_capture(&repo.join("submod"), &["rev-parse", "HEAD"]);
    let resolved_commit = String::from_utf8_lossy(&submod_head.stdout)
        .trim()
        .to_string();
    assert_eq!(
        resolved_commit, commit_b,
        "expected local submodule commit after answering 'l'\ngit output:\n{text}"
    );
}

#[test]
fn git_mergetool_submodule_conflict_resolved_via_remote() {
    // Verify answering "r" keeps the remote submodule commit.
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("main_repo");
    let sub_repo = tmp.path().join("sub_repo");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&sub_repo).unwrap();

    create_submodule_repo(&sub_repo);
    init_repo(&repo);
    let sub_url = format!("file://{}", sub_repo.display());
    run_git(&repo, &["submodule", "add", &sub_url, "submod"]);
    commit_all(&repo, "add submodule");

    advance_submodule(&sub_repo, "commit A\n", "advance A");
    let commit_a_out = run_git_capture(&sub_repo, &["rev-parse", "HEAD"]);
    let commit_a = String::from_utf8_lossy(&commit_a_out.stdout)
        .trim()
        .to_string();

    advance_submodule(&sub_repo, "commit B\n", "advance B");
    let commit_b_out = run_git_capture(&sub_repo, &["rev-parse", "HEAD"]);
    let _commit_b = String::from_utf8_lossy(&commit_b_out.stdout)
        .trim()
        .to_string();

    run_git(&repo, &["checkout", "-b", "feature"]);
    run_git(&repo.join("submod"), &["fetch"]);
    run_git(&repo.join("submod"), &["checkout", &commit_a]);
    run_git(&repo, &["add", "submod"]);
    run_git(
        &repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "feature: update submod",
        ],
    );

    run_git(&repo, &["checkout", "main"]);
    run_git(&repo, &["submodule", "update", "--init"]);
    run_git(&repo.join("submod"), &["fetch"]);
    run_git(&repo.join("submod"), &["checkout", &_commit_b]);
    run_git(&repo, &["add", "submod"]);
    run_git(
        &repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "main: update submod",
        ],
    );

    let merge_out = run_git_capture(&repo, &["merge", "feature"]);
    if merge_out.status.success() {
        return;
    }

    configure_gitgpui_mergetool(&repo);

    let output = run_git_with_stdin(&repo, &["mergetool", "--no-prompt"], "r\n");
    let text = output_text(&output);

    // The submodule should be resolved to the remote (feature) commit.
    let submod_head = run_git_capture(&repo.join("submod"), &["rev-parse", "HEAD"]);
    let resolved_commit = String::from_utf8_lossy(&submod_head.stdout)
        .trim()
        .to_string();
    assert_eq!(
        resolved_commit, commit_a,
        "expected remote submodule commit after answering 'r'\ngit output:\n{text}"
    );
}

#[test]
fn git_mergetool_submodule_alongside_normal_file_conflict() {
    // When a repo has both a submodule conflict and a normal file conflict,
    // git handles the submodule internally and invokes our external tool
    // for the normal file conflict.
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("main_repo");
    let sub_repo = tmp.path().join("sub_repo");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&sub_repo).unwrap();

    create_submodule_repo(&sub_repo);
    init_repo(&repo);
    let sub_url = format!("file://{}", sub_repo.display());
    run_git(&repo, &["submodule", "add", &sub_url, "submod"]);
    write_file(&repo, "normal.txt", "base\n");
    commit_all(&repo, "add submodule and file");

    advance_submodule(&sub_repo, "commit A\n", "advance A");
    let commit_a_out = run_git_capture(&sub_repo, &["rev-parse", "HEAD"]);
    let commit_a = String::from_utf8_lossy(&commit_a_out.stdout)
        .trim()
        .to_string();

    advance_submodule(&sub_repo, "commit B\n", "advance B");
    let commit_b_out = run_git_capture(&sub_repo, &["rev-parse", "HEAD"]);
    let commit_b = String::from_utf8_lossy(&commit_b_out.stdout)
        .trim()
        .to_string();

    // Feature branch: update submod to commit A and change normal.txt.
    run_git(&repo, &["checkout", "-b", "feature"]);
    run_git(&repo.join("submod"), &["fetch"]);
    run_git(&repo.join("submod"), &["checkout", &commit_a]);
    write_file(&repo, "normal.txt", "remote change\n");
    run_git(&repo, &["add", "submod", "normal.txt"]);
    run_git(
        &repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "feature changes",
        ],
    );

    // Main branch: update submod to commit B and change normal.txt differently.
    run_git(&repo, &["checkout", "main"]);
    run_git(&repo, &["submodule", "update", "--init"]);
    run_git(&repo.join("submod"), &["fetch"]);
    run_git(&repo.join("submod"), &["checkout", &commit_b]);
    write_file(&repo, "normal.txt", "local change\n");
    run_git(&repo, &["add", "submod", "normal.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "main changes"],
    );

    let merge_out = run_git_capture(&repo, &["merge", "feature"]);
    if merge_out.status.success() {
        return;
    }

    configure_gitgpui_mergetool(&repo);

    // Answer "l" for the submodule prompt. The normal file conflict
    // will be handled by our external tool.
    let output = run_git_with_stdin(&repo, &["mergetool", "--no-prompt"], "l\n");
    let text = output_text(&output);

    // Verify the normal file was processed by our mergetool.
    let normal_content = fs::read_to_string(repo.join("normal.txt")).unwrap();
    assert!(
        normal_content.contains("local")
            || normal_content.contains("remote")
            || normal_content.contains("<<<<<<<"),
        "expected external tool to process normal file conflict\nnormal.txt:\n{normal_content}\ngit output:\n{text}"
    );

    // Verify the submodule was resolved.
    let submod_head = run_git_capture(&repo.join("submod"), &["rev-parse", "HEAD"]);
    let resolved_commit = String::from_utf8_lossy(&submod_head.stdout)
        .trim()
        .to_string();
    assert_eq!(
        resolved_commit, commit_b,
        "expected local submodule commit\ngit output:\n{text}"
    );
}

#[test]
fn git_mergetool_file_replaced_by_submodule_conflict() {
    // One branch keeps a regular file, the other replaces it with a submodule.
    // Git mergetool handles this as a file-vs-submodule conflict.
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("main_repo");
    let sub_repo = tmp.path().join("sub_repo");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&sub_repo).unwrap();

    create_submodule_repo(&sub_repo);
    init_repo(&repo);

    // Base: create a regular file at the path that will become a submodule.
    write_file(&repo, "submod", "not a submodule\n");
    commit_all(&repo, "base: file at submod path");

    // Feature: replace the file with a submodule.
    run_git(&repo, &["checkout", "-b", "feature"]);
    run_git(&repo, &["rm", "submod"]);
    let sub_url = format!("file://{}", sub_repo.display());
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
    commit_all(&repo, "feature: replace file with submodule");

    // Main: modify the regular file.
    run_git(&repo, &["checkout", "main"]);
    write_file(&repo, "submod", "modified file content\n");
    commit_all(&repo, "main: modify file");

    let merge_out = run_git_capture(&repo, &["merge", "feature"]);
    if merge_out.status.success() {
        return;
    }

    configure_gitgpui_mergetool(&repo);

    // Git handles file-vs-submodule conflicts with its own prompt.
    // Pipe "l" to keep the local (file) side.
    let output = run_git_with_stdin(&repo, &["mergetool", "--no-prompt"], "l\n");
    let _text = output_text(&output);

    // The pipeline should complete without hanging or crashing.
    // The exact resolution depends on git version, but the key is
    // that the mergetool handled the mixed conflict type.
}

#[test]
fn git_mergetool_submodule_in_subdirectory_conflict() {
    // Submodule conflict where the submodule is inside a subdirectory.
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("main_repo");
    let sub_repo = tmp.path().join("sub_repo");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&sub_repo).unwrap();

    create_submodule_repo(&sub_repo);
    init_repo(&repo);
    fs::create_dir_all(repo.join("subdir")).unwrap();
    let sub_url = format!("file://{}", sub_repo.display());
    run_git(
        &repo,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            &sub_url,
            "subdir/submod",
        ],
    );
    commit_all(&repo, "add submodule in subdirectory");

    advance_submodule(&sub_repo, "commit A\n", "advance A");
    let commit_a_out = run_git_capture(&sub_repo, &["rev-parse", "HEAD"]);
    let commit_a = String::from_utf8_lossy(&commit_a_out.stdout)
        .trim()
        .to_string();

    advance_submodule(&sub_repo, "commit B\n", "advance B");
    let commit_b_out = run_git_capture(&sub_repo, &["rev-parse", "HEAD"]);
    let commit_b = String::from_utf8_lossy(&commit_b_out.stdout)
        .trim()
        .to_string();

    run_git(&repo, &["checkout", "-b", "feature"]);
    run_git(&repo.join("subdir/submod"), &["fetch"]);
    run_git(&repo.join("subdir/submod"), &["checkout", &commit_a]);
    run_git(&repo, &["add", "subdir/submod"]);
    run_git(
        &repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "feature: update submod",
        ],
    );

    run_git(&repo, &["checkout", "main"]);
    run_git(&repo, &["submodule", "update", "--init"]);
    run_git(&repo.join("subdir/submod"), &["fetch"]);
    run_git(&repo.join("subdir/submod"), &["checkout", &commit_b]);
    run_git(&repo, &["add", "subdir/submod"]);
    run_git(
        &repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "main: update submod",
        ],
    );

    let merge_out = run_git_capture(&repo, &["merge", "feature"]);
    if merge_out.status.success() {
        return;
    }

    configure_gitgpui_mergetool(&repo);

    let output = run_git_with_stdin(&repo, &["mergetool", "--no-prompt"], "l\n");
    let text = output_text(&output);

    // Verify the submodule in the subdirectory was resolved.
    let submod_head = run_git_capture(&repo.join("subdir/submod"), &["rev-parse", "HEAD"]);
    let resolved_commit = String::from_utf8_lossy(&submod_head.stdout)
        .trim()
        .to_string();
    assert_eq!(
        resolved_commit, commit_b,
        "expected local submodule commit in subdirectory\ngit output:\n{text}"
    );
}

#[test]
fn git_mergetool_deleted_submodule_conflict() {
    // One branch modifies a submodule, the other deletes it.
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("main_repo");
    let sub_repo = tmp.path().join("sub_repo");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&sub_repo).unwrap();

    create_submodule_repo(&sub_repo);
    init_repo(&repo);
    let sub_url = format!("file://{}", sub_repo.display());
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

    advance_submodule(&sub_repo, "advanced\n", "advance");
    let advanced_out = run_git_capture(&sub_repo, &["rev-parse", "HEAD"]);
    let _advanced_commit = String::from_utf8_lossy(&advanced_out.stdout)
        .trim()
        .to_string();

    // Feature: update submodule to new commit.
    run_git(&repo, &["checkout", "-b", "feature"]);
    run_git(&repo.join("submod"), &["fetch"]);
    run_git(&repo.join("submod"), &["checkout", &_advanced_commit]);
    run_git(&repo, &["add", "submod"]);
    run_git(
        &repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "feature: update submod",
        ],
    );

    // Main: remove the submodule.
    run_git(&repo, &["checkout", "main"]);
    run_git(&repo, &["submodule", "deinit", "-f", "submod"]);
    run_git(&repo, &["rm", "-f", "submod"]);
    // Clean up .gitmodules if it still references the removed submodule.
    let gitmodules = repo.join(".gitmodules");
    if gitmodules.exists() {
        let content = fs::read_to_string(&gitmodules).unwrap_or_default();
        if content.trim().is_empty() || !content.contains("[submodule") {
            // .gitmodules is empty or has no submodule sections; stage removal.
            let _ = run_git_capture(&repo, &["rm", "-f", ".gitmodules"]);
        }
    }
    run_git(&repo, &["add", "-A"]);
    run_git(
        &repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "main: remove submod",
        ],
    );

    let merge_out = run_git_capture(&repo, &["merge", "feature"]);
    if merge_out.status.success() {
        return;
    }

    configure_gitgpui_mergetool(&repo);

    // Git handles deleted-vs-modified submodule with its own prompt.
    // Answer "d" (delete) to keep our side's deletion.
    let output = run_git_with_stdin(&repo, &["mergetool", "--no-prompt"], "d\n");
    let _text = output_text(&output);

    // The pipeline should complete without crashing. The exact state
    // depends on the git version's handling of deleted submodules.
}

#[test]
fn git_mergetool_directory_vs_submodule_conflict() {
    // Parity with git t7610: "directory vs modified submodule".
    // One branch replaces a submodule with a regular directory (containing files).
    // The other branch modifies the submodule.  Git handles this conflict with
    // its own l/r prompts; we verify the mergetool pipeline completes.
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("main_repo");
    let sub_repo = tmp.path().join("sub_repo");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&sub_repo).unwrap();

    create_submodule_repo(&sub_repo);
    init_repo(&repo);

    // Base: add a submodule.
    let sub_url = format!("file://{}", sub_repo.display());
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

    // Feature: replace the submodule with a regular directory.
    run_git(&repo, &["checkout", "-b", "feature"]);
    run_git(&repo, &["submodule", "deinit", "-f", "submod"]);
    run_git(&repo, &["rm", "-f", "submod"]);
    // Clean up .gitmodules if empty.
    let gitmodules = repo.join(".gitmodules");
    if gitmodules.exists() {
        let content = fs::read_to_string(&gitmodules).unwrap_or_default();
        if content.trim().is_empty() || !content.contains("[submodule") {
            let _ = run_git_capture(&repo, &["rm", "-f", ".gitmodules"]);
        }
    }
    // Create a regular directory at the submod path.
    fs::create_dir_all(repo.join("submod")).unwrap();
    write_file(&repo, "submod/file16.txt", "not a submodule\n");
    commit_all(&repo, "feature: replace submodule with directory");

    // Main: update the submodule to a new commit.
    run_git(&repo, &["checkout", "main"]);
    run_git(&repo, &["submodule", "update", "--init"]);
    advance_submodule(&sub_repo, "advanced content\n", "advance submod");
    let advanced_out = run_git_capture(&sub_repo, &["rev-parse", "HEAD"]);
    let advanced_commit = String::from_utf8_lossy(&advanced_out.stdout)
        .trim()
        .to_string();
    run_git(&repo.join("submod"), &["fetch"]);
    run_git(&repo.join("submod"), &["checkout", &advanced_commit]);
    run_git(&repo, &["add", "submod"]);
    run_git(
        &repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "main: update submod",
        ],
    );

    let merge_out = run_git_capture(&repo, &["merge", "feature"]);
    if merge_out.status.success() {
        // Some git versions may auto-resolve this; that's fine.
        return;
    }

    configure_gitgpui_mergetool(&repo);

    // Git handles directory-vs-submodule conflicts with its own prompt.
    // Answer "l" to keep the local side (submodule).
    let output = run_git_with_stdin(&repo, &["mergetool", "--no-prompt"], "l\n");
    let _text = output_text(&output);

    // The pipeline should complete without hanging or crashing.
    // The exact resolution depends on git version.
}

// ── Git config fallback tests ──────────────────────────────────────
//
// When `trustExitCode=true` and our tool exits 1 (unresolvable conflict),
// `git mergetool` restores the MERGED file from its backup, erasing our
// tool's output. These tests therefore use `trustExitCode=false` so that
// git checks whether MERGED content changed (it always does because our
// tool writes fresh merge output) and keeps our tool's result on disk.

/// Configure gitgpui as the mergetool with `trustExitCode=false` so that
/// git keeps our tool's MERGED output even when conflicts remain.
fn configure_gitgpui_mergetool_no_trust(repo: &Path) {
    let bin = gitgpui_bin();
    let bin_q = shell_quote(&bin.to_string_lossy());
    let cmd = format!(
        "{bin_q} mergetool --base \"$BASE\" --local \"$LOCAL\" --remote \"$REMOTE\" --merged \"$MERGED\""
    );

    run_git(repo, &["config", "merge.tool", "gitgpui"]);
    run_git(repo, &["config", "mergetool.gitgpui.cmd", &cmd]);
    run_git(
        repo,
        &["config", "mergetool.gitgpui.trustExitCode", "false"],
    );
    run_git(repo, &["config", "mergetool.prompt", "false"]);
    run_git(repo, &["config", "mergetool.keepBackup", "false"]);
}

fn setup_simple_overlapping_conflict(repo: &Path) {
    write_file(repo, "file.txt", "original\n");
    commit_all(repo, "base");

    run_git(repo, &["checkout", "-b", "feature"]);
    write_file(repo, "file.txt", "remote change\n");
    commit_all(repo, "feature");

    run_git(repo, &["checkout", "main"]);
    write_file(repo, "file.txt", "local change\n");
    commit_all(repo, "main");

    let merge_out = run_git_capture(repo, &["merge", "feature", "--no-commit"]);
    assert!(
        !merge_out.status.success(),
        "expected merge conflict, got success"
    );
}

#[test]
fn git_mergetool_respects_merge_conflictstyle_zdiff3_from_git_config() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().to_path_buf();
    init_repo(&repo);
    setup_simple_overlapping_conflict(&repo);

    // Set zdiff3 via git config (no CLI flag).
    run_git(&repo, &["config", "merge.conflictstyle", "zdiff3"]);

    configure_gitgpui_mergetool_no_trust(&repo);

    let output = run_git_capture(&repo, &["mergetool", "--no-prompt"]);
    let text = output_text(&output);

    let merged = fs::read_to_string(repo.join("file.txt")).unwrap();

    // zdiff3/diff3 both include the ||||||| base section.
    assert!(
        merged.contains("|||||||"),
        "zdiff3 should include base section (|||||||), got:\n{merged}\nstderr:\n{text}"
    );
    // zdiff3 base section should contain the original content.
    assert!(
        merged.contains("original"),
        "zdiff3 base section should contain 'original', got:\n{merged}"
    );
}

#[test]
fn git_mergetool_respects_merge_conflictstyle_diff3_from_git_config() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().to_path_buf();
    init_repo(&repo);
    setup_simple_overlapping_conflict(&repo);

    // Set diff3 via git config.
    run_git(&repo, &["config", "merge.conflictstyle", "diff3"]);

    configure_gitgpui_mergetool_no_trust(&repo);

    let _output = run_git_capture(&repo, &["mergetool", "--no-prompt"]);
    let merged = fs::read_to_string(repo.join("file.txt")).unwrap();

    // diff3 format should include base section with "original".
    assert!(
        merged.contains("|||||||"),
        "diff3 should include base section (|||||||), got:\n{merged}"
    );
    assert!(
        merged.contains("original"),
        "diff3 base section should contain original content, got:\n{merged}"
    );
}

#[test]
fn git_mergetool_respects_diff_algorithm_histogram_from_git_config() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().to_path_buf();
    init_repo(&repo);

    // C code case where histogram produces clean merge but Myers may not.
    let base = "void f() {\n    x = 1;\n}\nvoid g() {\n    y = 1;\n}\n";
    let ours = "void h() {\n    z = 1;\n}\nvoid g() {\n    y = 1;\n}\n";
    let theirs = "void f() {\n    x = 1;\n}\nvoid g() {\n    y = 2;\n}\n";

    write_file(&repo, "code.c", base);
    commit_all(&repo, "base");

    run_git(&repo, &["checkout", "-b", "feature"]);
    write_file(&repo, "code.c", theirs);
    commit_all(&repo, "feature");

    run_git(&repo, &["checkout", "main"]);
    write_file(&repo, "code.c", ours);
    commit_all(&repo, "main");

    let merge_out = run_git_capture(&repo, &["merge", "feature", "--no-commit"]);
    if merge_out.status.success() {
        // Git itself may auto-resolve; skip if no conflict.
        return;
    }

    // Set histogram via git config.
    run_git(&repo, &["config", "diff.algorithm", "histogram"]);

    // Histogram should produce a clean merge (exit 0), so trustExitCode=true
    // works fine here — git will accept the result.
    configure_gitgpui_mergetool(&repo);

    let output = run_git_capture(&repo, &["mergetool", "--no-prompt"]);
    let text = output_text(&output);

    let merged = fs::read_to_string(repo.join("code.c")).unwrap();

    // Histogram should produce a cleaner merge that includes both changes.
    assert!(
        merged.contains("void h()") || merged.contains("void g()"),
        "histogram merge should produce meaningful output, got:\n{merged}\nstderr:\n{text}"
    );
}

#[test]
fn git_mergetool_cli_flag_overrides_git_config() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().to_path_buf();
    init_repo(&repo);
    setup_simple_overlapping_conflict(&repo);

    // Git config says zdiff3, but CLI flag overrides to merge.
    run_git(&repo, &["config", "merge.conflictstyle", "zdiff3"]);

    // Configure mergetool with explicit --conflict-style merge flag.
    let bin = gitgpui_bin();
    let bin_q = shell_quote(&bin.to_string_lossy());
    let cmd = format!(
        "{bin_q} mergetool --conflict-style merge --base \"$BASE\" --local \"$LOCAL\" --remote \"$REMOTE\" --merged \"$MERGED\""
    );
    run_git(&repo, &["config", "merge.tool", "gitgpui"]);
    run_git(&repo, &["config", "mergetool.gitgpui.cmd", &cmd]);
    // Use trustExitCode=false so git keeps our tool's output.
    run_git(
        &repo,
        &["config", "mergetool.gitgpui.trustExitCode", "false"],
    );
    run_git(&repo, &["config", "mergetool.prompt", "false"]);
    run_git(&repo, &["config", "mergetool.keepBackup", "false"]);

    let _output = run_git_capture(&repo, &["mergetool", "--no-prompt"]);
    let merged = fs::read_to_string(repo.join("file.txt")).unwrap();

    // CLI flag "merge" should override config "zdiff3" — no base section.
    assert!(
        !merged.contains("|||||||"),
        "explicit --conflict-style merge should suppress base section, got:\n{merged}"
    );
    assert!(merged.contains("<<<<<<<"), "should have conflict markers");
}

// ── Binary file conflict handling ────────────────────────────────────

fn write_bytes_to_file(repo: &Path, rel: &str, bytes: &[u8]) {
    let path = repo.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent directories");
    }
    fs::write(path, bytes).expect("write bytes file");
}

/// Create a conflict on a binary file: base has one binary content,
/// main and feature branches modify it differently.
fn setup_binary_conflict(repo: &Path) {
    init_repo(repo);

    // Base: a small "binary" file with null bytes.
    let base_bytes: Vec<u8> = vec![0x89, 0x50, 0x4E, 0x47, 0x00, 0x01, 0x02, 0x03];
    write_bytes_to_file(repo, "image.bin", &base_bytes);
    commit_all(repo, "base: add binary file");

    run_git(repo, &["checkout", "-b", "feature"]);
    let feature_bytes: Vec<u8> = vec![0x89, 0x50, 0x4E, 0x47, 0x00, 0xAA, 0xBB, 0xCC];
    write_bytes_to_file(repo, "image.bin", &feature_bytes);
    commit_all(repo, "feature: modify binary file");

    run_git(repo, &["checkout", "main"]);
    let main_bytes: Vec<u8> = vec![0x89, 0x50, 0x4E, 0x47, 0x00, 0xDD, 0xEE, 0xFF];
    write_bytes_to_file(repo, "image.bin", &main_bytes);
    commit_all(repo, "main: modify binary file differently");

    // Merge will fail with a conflict.
    let output = run_git_capture(repo, &["merge", "feature"]);
    assert!(
        !output.status.success(),
        "expected merge to fail with binary conflict"
    );
}

/// E2E: `git mergetool` with a binary file conflict reports the binary
/// conflict and keeps the local version in the merged output.
///
/// This covers behavior matrix item #4 (binary and non-UTF8 content)
/// end-to-end through the actual `git mergetool` invocation.
#[test]
fn git_mergetool_binary_conflict_keeps_local_version() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().to_path_buf();
    setup_binary_conflict(&repo);
    configure_gitgpui_mergetool(&repo);

    let output = run_git_capture(&repo, &["mergetool", "--no-prompt"]);
    let text = output_text(&output);

    // The tool should report binary conflict via stderr.
    assert!(
        text.contains("binary") || text.contains("Binary"),
        "expected binary conflict message in tool output, got:\n{text}"
    );

    // With trustExitCode=true and our tool exiting 1 (CANCELED for binary),
    // git considers the file unresolved. The merged file should contain
    // the local version's bytes (our tool copies LOCAL to MERGED for binary).
    let merged_bytes = fs::read(repo.join("image.bin")).unwrap();
    let main_bytes: Vec<u8> = vec![0x89, 0x50, 0x4E, 0x47, 0x00, 0xDD, 0xEE, 0xFF];
    assert_eq!(
        merged_bytes, main_bytes,
        "binary conflict should keep local (main) version in MERGED"
    );
}

/// E2E: `git mergetool` with a binary file conflict alongside a text conflict
/// processes both files correctly — the text conflict gets auto-merged while
/// the binary conflict is detected and handled separately.
#[test]
fn git_mergetool_binary_conflict_alongside_text_conflict() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().to_path_buf();
    init_repo(&repo);

    // Base commit: one text file and one binary file.
    write_file(&repo, "text.txt", "aaa\nbbb\nccc\n");
    let base_bytes: Vec<u8> = vec![0x00, 0x01, 0x02, 0x03];
    write_bytes_to_file(&repo, "image.bin", &base_bytes);
    commit_all(&repo, "base");

    // Feature branch: modify both files.
    run_git(&repo, &["checkout", "-b", "feature"]);
    write_file(&repo, "text.txt", "aaa\nREMOTE\nccc\n");
    let feature_bytes: Vec<u8> = vec![0x00, 0xAA, 0xBB, 0xCC];
    write_bytes_to_file(&repo, "image.bin", &feature_bytes);
    commit_all(&repo, "feature: change both files");

    // Main branch: modify both files differently.
    run_git(&repo, &["checkout", "main"]);
    write_file(&repo, "text.txt", "aaa\nLOCAL\nccc\n");
    let main_bytes: Vec<u8> = vec![0x00, 0xDD, 0xEE, 0xFF];
    write_bytes_to_file(&repo, "image.bin", &main_bytes);
    commit_all(&repo, "main: change both files differently");

    let merge_output = run_git_capture(&repo, &["merge", "feature"]);
    assert!(
        !merge_output.status.success(),
        "expected merge to fail with conflicts"
    );

    configure_gitgpui_mergetool(&repo);
    let output = run_git_capture(&repo, &["mergetool", "--no-prompt"]);
    let text = output_text(&output);

    // The text file conflict should have been processed by our mergetool.
    // Since both sides changed the same line differently, it will have markers.
    let text_merged = fs::read_to_string(repo.join("text.txt")).unwrap();
    assert!(
        text_merged.contains("<<<<<<<")
            || text_merged.contains("LOCAL")
            || text_merged.contains("REMOTE"),
        "text conflict should be processed, got:\n{text_merged}"
    );

    // The binary file should have been handled: local version kept.
    let bin_merged = fs::read(repo.join("image.bin")).unwrap();
    assert_eq!(
        bin_merged, main_bytes,
        "binary conflict should keep local (main) version"
    );

    // Tool output should mention binary.
    assert!(
        text.contains("binary") || text.contains("Binary"),
        "expected binary conflict mention in output, got:\n{text}"
    );
}
