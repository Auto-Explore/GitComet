use gitcomet_core::domain::CommitId;
use gitcomet_core::services::GitBackend;
use gitcomet_git_gix::GixBackend;
#[path = "support/test_git_env.rs"]
mod test_git_env;
use std::fs;
use std::path::Path;
use std::process::Command;

fn git_command() -> Command {
    let mut cmd = Command::new("git");
    // Keep tests deterministic by isolating from host git config.
    test_git_env::apply(&mut cmd);
    cmd
}

fn run_git(repo: &Path, args: &[&str]) {
    let status = git_command()
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .expect("git command to run");
    assert!(status.success(), "git {:?} failed", args);
}

fn run_git_capture(repo: &Path, args: &[&str]) -> String {
    let output = git_command()
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("git command to run");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[test]
fn export_patch_and_apply_patch_round_trip() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
    run_git(repo, &["config", "core.autocrlf", "false"]);
    run_git(repo, &["config", "core.eol", "lf"]);

    let note = repo.join("note.txt");
    fs::write(&note, "one\n").expect("write initial file");
    run_git(repo, &["add", "note.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    fs::write(&note, "one\ntwo\n").expect("write modified file");
    run_git(repo, &["add", "note.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "add line"],
    );

    let head = run_git_capture(repo, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    let patch_path = dir.path().join("change.patch");

    let backend = GixBackend;
    let opened = backend.open(repo).expect("open repository");

    let export_output = opened
        .export_patch_with_output(&CommitId(head.into()), &patch_path)
        .expect("export patch");
    assert_eq!(export_output.exit_code, Some(0));
    assert!(patch_path.exists(), "expected patch file to exist");

    let patch_text = fs::read_to_string(&patch_path).expect("read patch file");
    assert!(patch_text.contains("Subject: [PATCH] add line"));
    assert!(patch_text.contains("+two"));

    run_git(repo, &["reset", "--hard", "HEAD~1"]);
    assert_eq!(
        fs::read_to_string(&note).expect("read reset file"),
        "one\n",
        "reset should remove second line"
    );

    let apply_output = opened
        .apply_patch_with_output(&patch_path)
        .expect("apply exported patch");
    assert_eq!(apply_output.exit_code, Some(0));
    assert_eq!(
        fs::read_to_string(&note).expect("read applied file"),
        "one\ntwo\n"
    );

    let subject = run_git_capture(repo, &["log", "-1", "--pretty=%s"])
        .trim()
        .to_string();
    assert_eq!(subject, "add line");
}

#[test]
fn apply_unified_patch_to_worktree_applies_and_reverses() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
    run_git(repo, &["config", "core.autocrlf", "false"]);
    run_git(repo, &["config", "core.eol", "lf"]);

    let file = repo.join("a.txt");
    fs::write(&file, "base\n").expect("write base file");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    fs::write(&file, "base\nchanged\n").expect("write modified file");
    let patch = run_git_capture(repo, &["diff", "--", "a.txt"]);
    assert!(patch.contains("+changed"));

    run_git(repo, &["checkout", "--", "a.txt"]);
    assert_eq!(
        fs::read_to_string(&file).expect("read restored file"),
        "base\n"
    );

    let backend = GixBackend;
    let opened = backend.open(repo).expect("open repository");

    let apply_output = opened
        .apply_unified_patch_to_worktree_with_output(&patch, false)
        .expect("apply worktree patch");
    assert_eq!(apply_output.exit_code, Some(0));
    assert!(
        apply_output.command.starts_with("git apply "),
        "unexpected command label: {}",
        apply_output.command
    );
    assert_eq!(
        fs::read_to_string(&file).expect("read patched file"),
        "base\nchanged\n"
    );

    let reverse_output = opened
        .apply_unified_patch_to_worktree_with_output(&patch, true)
        .expect("reverse worktree patch");
    assert_eq!(reverse_output.exit_code, Some(0));
    assert!(
        reverse_output.command.starts_with("git apply --reverse "),
        "unexpected command label: {}",
        reverse_output.command
    );
    assert_eq!(
        fs::read_to_string(&file).expect("read reversed file"),
        "base\n"
    );
}
