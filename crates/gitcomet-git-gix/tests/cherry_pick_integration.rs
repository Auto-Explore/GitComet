use gitcomet_core::domain::CommitId;
use gitcomet_core::services::{
    GitBackend, GitRepository, InteractiveRebaseAction, InteractiveRebaseEntry, SequencerState,
};
use gitcomet_git_gix::GixBackend;
#[path = "support/test_git_env.rs"]
mod test_git_env;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

#[cfg(unix)]
fn install_prepare_commit_msg_hook(repo: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt as _;

    let hook = repo.join(".git").join("hooks").join("prepare-commit-msg");
    fs::write(&hook, script).expect("write prepare-commit-msg hook");
    let mut permissions = fs::metadata(&hook).expect("stat hook").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(hook, permissions).expect("make hook executable");
}

fn run_git(repo: &Path, args: &[&str]) {
    let mut cmd = Command::new("git");
    test_git_env::apply(&mut cmd);
    let status = cmd
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("EDITOR", "true")
        .env("VISUAL", "true")
        .status()
        .expect("git command to run");
    assert!(status.success(), "git {:?} failed", args);
}

fn git_output(repo: &Path, args: &[&str]) -> std::process::Output {
    let mut cmd = Command::new("git");
    test_git_env::apply(&mut cmd);
    cmd.arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("EDITOR", "true")
        .env("VISUAL", "true")
        .output()
        .expect("git command to run")
}

fn git_stdout(repo: &Path, args: &[&str]) -> String {
    let output = git_output(repo, args);
    assert!(output.status.success(), "git {:?} failed", args);
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn init_repo(repo: &Path) {
    fs::create_dir_all(repo).expect("create repo directory");
    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
}

fn commit_file(repo: &Path, name: &str, content: &str, message: &str) -> String {
    fs::write(repo.join(name), content).expect("write file");
    run_git(repo, &["add", "."]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", message],
    );
    git_stdout(repo, &["rev-parse", "HEAD"])
}

fn commit_id(sha: &str) -> CommitId {
    CommitId(sha.into())
}

fn open_backend(repo: &Path) -> Arc<dyn GitRepository> {
    GixBackend.open(repo).expect("open repository")
}

/// Creates a merge whose first-parent-only and second-parent-only changes are
/// distinguishable, then checks out `target` at the common base.
fn setup_merge_cherry_pick_repo(repo: &Path) -> (String, String) {
    init_repo(repo);
    let base = commit_file(repo, "base.txt", "base\n", "base");
    run_git(repo, &["checkout", "-b", "source"]);
    commit_file(repo, "mainline.txt", "mainline\n", "mainline change");
    run_git(repo, &["checkout", "-b", "side", &base]);
    commit_file(repo, "side.txt", "side\n", "side change");
    run_git(repo, &["checkout", "source"]);
    run_git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "merge",
            "--no-ff",
            "side",
            "-m",
            "merge side",
        ],
    );
    let merge = git_stdout(repo, &["rev-parse", "HEAD"]);
    run_git(repo, &["checkout", "-b", "target", &base]);
    (base, merge)
}

fn setup_octopus_cherry_pick_repo(repo: &Path) -> String {
    init_repo(repo);
    let base = commit_file(repo, "base.txt", "base\n", "base");
    run_git(repo, &["checkout", "-b", "source"]);
    commit_file(repo, "mainline.txt", "mainline\n", "mainline change");
    run_git(repo, &["checkout", "-b", "side-two", &base]);
    commit_file(repo, "side-two.txt", "side two\n", "side two change");
    run_git(repo, &["checkout", "-b", "side-three", &base]);
    commit_file(repo, "side-three.txt", "side three\n", "side three change");
    run_git(repo, &["checkout", "source"]);
    run_git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "merge",
            "side-two",
            "side-three",
            "-m",
            "merge sides",
        ],
    );
    let merge = git_stdout(repo, &["rev-parse", "HEAD"]);
    run_git(repo, &["checkout", "-b", "target", &base]);
    merge
}

#[test]
fn single_cherry_pick_with_commit_creates_commit() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "base.txt", "base\n", "base");
    run_git(&repo, &["checkout", "-b", "feature"]);
    let picked = commit_file(&repo, "feature.txt", "feature\n", "feature change");
    run_git(&repo, &["checkout", "main"]);
    commit_file(&repo, "main.txt", "main\n", "main change");
    let before_count = git_stdout(&repo, &["rev-list", "--count", "HEAD"]);

    let output = open_backend(&repo)
        .cherry_pick_with_output(&commit_id(&picked), true, None)
        .expect("cherry-pick");

    assert_eq!(output.exit_code, Some(0));
    assert_eq!(
        git_stdout(&repo, &["rev-list", "--count", "HEAD"]),
        (before_count.parse::<u32>().unwrap() + 1).to_string()
    );
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%s"]),
        "feature change"
    );
    assert_eq!(
        fs::read_to_string(repo.join("feature.txt")).unwrap(),
        "feature\n"
    );
}

#[test]
fn single_cherry_pick_without_commit_applies_index_and_worktree_only() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "base.txt", "base\n", "base");
    run_git(&repo, &["checkout", "-b", "feature"]);
    let picked = commit_file(&repo, "feature.txt", "feature\n", "feature change");
    run_git(&repo, &["checkout", "main"]);
    let before_head = git_stdout(&repo, &["rev-parse", "HEAD"]);

    let output = open_backend(&repo)
        .cherry_pick_with_output(&commit_id(&picked), false, None)
        .expect("cherry-pick --no-commit");

    assert_eq!(output.exit_code, Some(0));
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), before_head);
    assert_eq!(
        git_stdout(&repo, &["status", "--porcelain"]),
        "A  feature.txt"
    );
    assert_eq!(
        fs::read_to_string(repo.join("feature.txt")).unwrap(),
        "feature\n"
    );
}

#[test]
fn single_merge_cherry_pick_uses_selected_mainline_parent() {
    for (mainline, expected_file, absent_file) in [
        (1, "side.txt", "mainline.txt"),
        (2, "mainline.txt", "side.txt"),
    ] {
        let dir = tempfile::tempdir().expect("create tempdir");
        let repo = dir.path().join("repo");
        let (_base, merge) = setup_merge_cherry_pick_repo(&repo);

        let output = open_backend(&repo)
            .cherry_pick_with_output(&commit_id(&merge), true, Some(mainline))
            .expect("cherry-pick merge");

        assert_eq!(output.exit_code, Some(0));
        assert!(
            output
                .command
                .contains(&format!("git cherry-pick -m {mainline}")),
            "unexpected command label: {}",
            output.command
        );
        assert_eq!(
            git_stdout(&repo, &["log", "-1", "--format=%s"]),
            "merge side"
        );
        assert!(repo.join(expected_file).is_file());
        assert!(!repo.join(absent_file).exists());
    }
}

#[test]
fn single_octopus_merge_accepts_every_parent_as_mainline() {
    let parent_files = ["mainline.txt", "side-two.txt", "side-three.txt"];
    for mainline in 1..=parent_files.len() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let repo = dir.path().join("repo");
        let merge = setup_octopus_cherry_pick_repo(&repo);

        open_backend(&repo)
            .cherry_pick_with_output(&commit_id(&merge), true, Some(mainline))
            .expect("cherry-pick octopus merge");

        for (ix, file) in parent_files.iter().enumerate() {
            assert_eq!(
                repo.join(file).exists(),
                ix + 1 != mainline,
                "mainline {mainline} should apply only the other parents' changes"
            );
        }
    }
}

#[test]
fn single_merge_cherry_pick_without_commit_uses_selected_mainline() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let (_base, merge) = setup_merge_cherry_pick_repo(&repo);
    let before_head = git_stdout(&repo, &["rev-parse", "HEAD"]);

    let output = open_backend(&repo)
        .cherry_pick_with_output(&commit_id(&merge), false, Some(1))
        .expect("cherry-pick merge --no-commit");

    assert_eq!(output.exit_code, Some(0));
    assert!(output.command.contains("git cherry-pick -m 1 --no-commit"));
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), before_head);
    assert_eq!(git_stdout(&repo, &["status", "--porcelain"]), "A  side.txt");
    assert!(!repo.join("mainline.txt").exists());
}

#[test]
fn single_cherry_pick_validates_mainline_before_starting() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let (base, merge) = setup_merge_cherry_pick_repo(&repo);
    let before_head = git_stdout(&repo, &["rev-parse", "HEAD"]);

    for (mainline, expected) in [
        (
            None,
            "is a merge commit with 2 parents; choose a mainline parent",
        ),
        (Some(0), "mainline parent 0 is invalid for merge commit"),
        (Some(3), "mainline parent 3 is invalid for merge commit"),
    ] {
        let err = open_backend(&repo)
            .cherry_pick_with_output(&commit_id(&merge), true, mainline)
            .expect_err("invalid merge mainline should be rejected");
        assert!(
            err.to_string().contains(expected),
            "unexpected error: {err}"
        );
        assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), before_head);
        assert_eq!(git_stdout(&repo, &["status", "--porcelain"]), "");
        assert_eq!(
            open_backend(&repo).sequencer_state().unwrap(),
            SequencerState::None
        );
    }

    let err = open_backend(&repo)
        .cherry_pick_with_output(&commit_id(&base), true, Some(1))
        .expect_err("non-merge mainline should be rejected");
    assert!(
        err.to_string()
            .contains("is not a merge commit; a mainline parent cannot be selected"),
        "unexpected error: {err}"
    );
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), before_head);
    assert_eq!(git_stdout(&repo, &["status", "--porcelain"]), "");
}

#[test]
fn already_applied_cherry_pick_is_successful_noop_and_cleans_state() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "file.txt", "old\n", "base");
    run_git(&repo, &["checkout", "-b", "feature"]);
    let picked = commit_file(&repo, "file.txt", "new\n", "same change");
    run_git(&repo, &["checkout", "main"]);
    commit_file(&repo, "file.txt", "new\n", "same change independently");
    let before_head = git_stdout(&repo, &["rev-parse", "HEAD"]);

    let output = open_backend(&repo)
        .cherry_pick_with_output(&commit_id(&picked), true, None)
        .expect("already-applied cherry-pick");

    assert_eq!(output.exit_code, Some(0));
    assert!(
        output
            .stdout
            .contains("GITCOMET_CHERRY_PICK_ALREADY_APPLIED")
    );
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), before_head);
    assert_eq!(git_stdout(&repo, &["status", "--porcelain"]), "");
    assert!(!open_backend(&repo).rebase_in_progress().unwrap());
    assert!(
        !git_output(&repo, &["rev-parse", "-q", "--verify", "CHERRY_PICK_HEAD"])
            .status
            .success()
    );
}

#[test]
fn already_applied_merge_cherry_pick_uses_selected_parent_for_empty_detection() {
    for (mainline, already_applied_file, contents) in
        [(1, "side.txt", "side\n"), (2, "mainline.txt", "mainline\n")]
    {
        let dir = tempfile::tempdir().expect("create tempdir");
        let repo = dir.path().join("repo");
        let (_base, merge) = setup_merge_cherry_pick_repo(&repo);
        commit_file(
            &repo,
            already_applied_file,
            contents,
            "same merge change independently",
        );
        let before_head = git_stdout(&repo, &["rev-parse", "HEAD"]);

        let output = open_backend(&repo)
            .cherry_pick_with_output(&commit_id(&merge), true, Some(mainline))
            .expect("already-applied merge should be a successful no-op");

        assert!(
            output
                .stdout
                .contains("GITCOMET_CHERRY_PICK_ALREADY_APPLIED")
        );
        assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), before_head);
        assert!(!open_backend(&repo).rebase_in_progress().unwrap());
        assert!(!repo.join(".git/gitcomet-cherry-pick-mainline").exists());
    }
}

#[test]
fn merge_conflict_resolved_to_no_changes_is_skipped_on_continue() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    let base = commit_file(&repo, "file.txt", "base\n", "base");
    run_git(&repo, &["checkout", "-b", "source"]);
    commit_file(&repo, "file.txt", "mainline\n", "mainline change");
    run_git(&repo, &["checkout", "-b", "side", &base]);
    commit_file(&repo, "file.txt", "side\n", "side change");
    run_git(&repo, &["checkout", "source"]);
    let merge_output = git_output(
        &repo,
        &["-c", "commit.gpgsign=false", "merge", "--no-ff", "side"],
    );
    assert!(!merge_output.status.success(), "merge should conflict");
    fs::write(repo.join("file.txt"), "merged\n").expect("resolve source merge");
    run_git(&repo, &["add", "file.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "merge side"],
    );
    let merge = git_stdout(&repo, &["rev-parse", "HEAD"]);
    run_git(&repo, &["checkout", "-b", "target", &base]);
    commit_file(&repo, "file.txt", "target\n", "target change");
    let before_head = git_stdout(&repo, &["rev-parse", "HEAD"]);

    open_backend(&repo)
        .cherry_pick_with_output(&commit_id(&merge), true, Some(1))
        .expect_err("merge cherry-pick should pause at a conflict");
    assert!(repo.join(".git/gitcomet-cherry-pick-mainline").is_file());
    fs::write(repo.join("file.txt"), "target\n").expect("keep target resolution");
    run_git(&repo, &["add", "file.txt"]);

    open_backend(&repo)
        .rebase_continue_with_output()
        .expect("empty merge resolution should be skipped");

    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), before_head);
    assert!(!open_backend(&repo).rebase_in_progress().unwrap());
    assert!(!repo.join(".git/gitcomet-cherry-pick-mainline").exists());
}

#[test]
fn interactive_reword_without_changed_message_uses_original_message() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "base.txt", "base\n", "base");
    run_git(&repo, &["checkout", "-b", "feature"]);
    let picked = commit_file(&repo, "feature.txt", "feature\n", "feature change");
    run_git(&repo, &["checkout", "main"]);

    open_backend(&repo)
        .interactive_cherry_pick_with_output(&[InteractiveRebaseEntry {
            action: InteractiveRebaseAction::Reword,
            commit_id: picked,
            summary: "feature change".to_string(),
            message: "feature change".to_string(),
            new_message: None,
        }])
        .expect("reword without edited message should use original message");

    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%s"]),
        "feature change"
    );
}

#[test]
fn custom_cherry_pick_rejects_staged_changes() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "base.txt", "base\n", "base");
    run_git(&repo, &["checkout", "-b", "feature"]);
    let picked = commit_file(&repo, "feature.txt", "feature\n", "feature change");
    run_git(&repo, &["checkout", "main"]);
    let before_head = git_stdout(&repo, &["rev-parse", "HEAD"]);
    fs::write(repo.join("unrelated.txt"), "staged work\n").expect("write staged file");
    run_git(&repo, &["add", "unrelated.txt"]);

    let err = open_backend(&repo)
        .interactive_cherry_pick_with_output(&[InteractiveRebaseEntry {
            action: InteractiveRebaseAction::Reword,
            commit_id: picked,
            summary: "feature change".to_string(),
            message: "feature change".to_string(),
            new_message: Some("reworded".to_string()),
        }])
        .expect_err("staged changes should reject a custom cherry-pick");

    let message = err.to_string();
    assert!(
        message.contains("uncommitted changes") || message.contains("unstaged"),
        "unexpected dirty-index error: {message}"
    );
    // The staged work is untouched and nothing was committed or left in
    // progress.
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), before_head);
    assert_eq!(
        git_stdout(&repo, &["status", "--porcelain"]),
        "A  unrelated.txt"
    );
    assert!(!open_backend(&repo).rebase_in_progress().unwrap());
}

#[test]
fn custom_cherry_pick_resumes_full_plan_after_conflict() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "file.txt", "base\n", "base");
    run_git(&repo, &["checkout", "-b", "feature"]);
    let conflicting = commit_file(&repo, "file.txt", "feature\n", "feature change");
    let reworded = commit_file(&repo, "second.txt", "second\n", "second change");
    run_git(&repo, &["checkout", "main"]);
    commit_file(&repo, "file.txt", "main\n", "main change");

    let output = open_backend(&repo)
        .interactive_cherry_pick_with_output(&[
            InteractiveRebaseEntry {
                action: InteractiveRebaseAction::Pick,
                commit_id: conflicting,
                summary: "feature change".to_string(),
                message: "feature change".to_string(),
                new_message: None,
            },
            InteractiveRebaseEntry {
                action: InteractiveRebaseAction::Reword,
                commit_id: reworded,
                summary: "second change".to_string(),
                message: "second change".to_string(),
                new_message: Some("second reworded".to_string()),
            },
        ])
        .expect("conflicting custom cherry-pick should pause, not fail");
    assert_ne!(output.exit_code, Some(0));
    assert!(open_backend(&repo).rebase_in_progress().unwrap());
    assert_eq!(git_stdout(&repo, &["status", "--porcelain"]), "UU file.txt");

    fs::write(repo.join("file.txt"), "resolved\n").expect("resolve conflict");
    run_git(&repo, &["add", "file.txt"]);

    open_backend(&repo)
        .rebase_continue_with_output()
        .expect("continue should finish the remaining plan");

    assert!(!open_backend(&repo).rebase_in_progress().unwrap());
    assert_eq!(
        git_stdout(&repo, &["log", "-2", "--format=%s"]),
        "second reworded\nfeature change"
    );
    assert_eq!(
        fs::read_to_string(repo.join("second.txt")).unwrap(),
        "second\n"
    );
}

#[test]
fn custom_cherry_pick_drops_already_applied_commits() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "shared.txt", "old\n", "base");
    run_git(&repo, &["checkout", "-b", "feature"]);
    let applied = commit_file(&repo, "shared.txt", "new\n", "same change");
    let reworded = commit_file(&repo, "extra.txt", "extra\n", "extra change");
    run_git(&repo, &["checkout", "main"]);
    commit_file(&repo, "shared.txt", "new\n", "same change independently");
    let before_count: u32 = git_stdout(&repo, &["rev-list", "--count", "HEAD"])
        .parse()
        .unwrap();

    open_backend(&repo)
        .interactive_cherry_pick_with_output(&[
            InteractiveRebaseEntry {
                action: InteractiveRebaseAction::Pick,
                commit_id: applied,
                summary: "same change".to_string(),
                message: "same change".to_string(),
                new_message: None,
            },
            InteractiveRebaseEntry {
                action: InteractiveRebaseAction::Reword,
                commit_id: reworded,
                summary: "extra change".to_string(),
                message: "extra change".to_string(),
                new_message: Some("extra reworded".to_string()),
            },
        ])
        .expect("already-applied pick should be dropped, not strand the plan");

    assert!(!open_backend(&repo).rebase_in_progress().unwrap());
    assert_eq!(
        git_stdout(&repo, &["rev-list", "--count", "HEAD"]),
        (before_count + 1).to_string()
    );
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%s"]),
        "extra reworded"
    );
    assert_eq!(
        fs::read_to_string(repo.join("extra.txt")).unwrap(),
        "extra\n"
    );
}

#[test]
fn empty_fold_anchor_never_rewrites_the_target_commit() {
    for fold_action in [
        InteractiveRebaseAction::Squash,
        InteractiveRebaseAction::Fixup,
    ] {
        let dir = tempfile::tempdir().expect("create tempdir");
        let repo = dir.path().join("repo");
        init_repo(&repo);
        commit_file(&repo, "shared.txt", "old\n", "base");
        run_git(&repo, &["checkout", "-b", "feature"]);
        let anchor = commit_file(&repo, "shared.txt", "new\n", "anchor change");
        let folded = commit_file(&repo, "folded.txt", "folded\n", "folded change");
        run_git(&repo, &["checkout", "main"]);
        commit_file(&repo, "shared.txt", "new\n", "anchor change independently");
        let target_head = git_stdout(&repo, &["rev-parse", "HEAD"]);
        let before_count: u32 = git_stdout(&repo, &["rev-list", "--count", "HEAD"])
            .parse()
            .unwrap();

        open_backend(&repo)
            .interactive_cherry_pick_with_output(&[
                InteractiveRebaseEntry {
                    action: InteractiveRebaseAction::Pick,
                    commit_id: anchor,
                    summary: "anchor change".to_string(),
                    message: "anchor change".to_string(),
                    new_message: None,
                },
                InteractiveRebaseEntry {
                    action: fold_action,
                    commit_id: folded,
                    summary: "folded change".to_string(),
                    message: "folded change".to_string(),
                    new_message: None,
                },
            ])
            .expect("empty fold anchor should be preserved");

        assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD^"]), target_head);
        assert_eq!(
            git_stdout(&repo, &["rev-list", "--count", "HEAD"]),
            (before_count + 1).to_string()
        );
        assert_eq!(
            fs::read_to_string(repo.join("folded.txt")).unwrap(),
            "folded\n"
        );
    }
}

#[test]
fn fold_plan_preserves_other_newly_empty_picks() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "shared.txt", "old\n", "base");
    run_git(&repo, &["checkout", "-b", "feature"]);
    let already_applied = commit_file(&repo, "shared.txt", "new\n", "already applied");
    let anchor = commit_file(&repo, "anchor.txt", "anchor\n", "fold anchor");
    let folded = commit_file(&repo, "folded.txt", "folded\n", "folded change");
    run_git(&repo, &["checkout", "main"]);
    commit_file(&repo, "shared.txt", "new\n", "same change independently");
    let before_count: u32 = git_stdout(&repo, &["rev-list", "--count", "HEAD"])
        .parse()
        .unwrap();
    let entry = |action, commit_id: String, message: &str| InteractiveRebaseEntry {
        action,
        commit_id,
        summary: message.to_string(),
        message: message.to_string(),
        new_message: None,
    };

    open_backend(&repo)
        .interactive_cherry_pick_with_output(&[
            entry(
                InteractiveRebaseAction::Pick,
                already_applied,
                "already applied",
            ),
            entry(InteractiveRebaseAction::Pick, anchor, "fold anchor"),
            entry(InteractiveRebaseAction::Fixup, folded, "folded change"),
        ])
        .expect("fold plan should keep every newly-empty pick");

    assert_eq!(
        git_stdout(&repo, &["rev-list", "--count", "HEAD"]),
        (before_count + 2).to_string()
    );
    assert_eq!(
        git_stdout(&repo, &["log", "-2", "--format=%s"]),
        "fold anchor\nalready applied"
    );
}

#[test]
fn selected_transitive_ancestors_sort_before_descendants() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    let base = commit_file(&repo, "base.txt", "base\n", "base");
    run_git(&repo, &["checkout", "-b", "chain"]);
    let ancestor = commit_file(&repo, "a.txt", "a\n", "ancestor");
    commit_file(&repo, "b.txt", "b\n", "unselected middle");
    let descendant = commit_file(&repo, "c.txt", "c\n", "descendant");
    run_git(&repo, &["checkout", "-b", "unrelated", &base]);
    let unrelated = commit_file(&repo, "x.txt", "x\n", "unrelated");

    let ordered = open_backend(&repo)
        .topologically_order_commits(&[
            commit_id(&descendant),
            commit_id(&unrelated),
            commit_id(&ancestor),
        ])
        .expect("order selected commits through unselected ancestry");
    let ordered = ordered.iter().map(|id| id.as_ref()).collect::<Vec<_>>();

    assert_eq!(
        ordered,
        [unrelated.as_str(), ancestor.as_str(), descendant.as_str()]
    );
}

#[test]
fn multi_cherry_pick_skips_already_applied_commits() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "shared.txt", "old\n", "base");
    run_git(&repo, &["checkout", "-b", "feature"]);
    let applied = commit_file(&repo, "shared.txt", "new\n", "same change");
    let fresh = commit_file(&repo, "fresh.txt", "fresh\n", "fresh change");
    run_git(&repo, &["checkout", "main"]);
    commit_file(&repo, "shared.txt", "new\n", "same change independently");
    let before_count: u32 = git_stdout(&repo, &["rev-list", "--count", "HEAD"])
        .parse()
        .unwrap();

    let pick = |commit_id: String, subject: &str| InteractiveRebaseEntry {
        action: InteractiveRebaseAction::Pick,
        commit_id,
        summary: subject.to_string(),
        message: subject.to_string(),
        new_message: None,
    };
    let output = open_backend(&repo)
        .interactive_cherry_pick_with_output(&[
            pick(applied, "same change"),
            pick(fresh, "fresh change"),
        ])
        .expect("empty pick should be skipped, not strand the sequence");

    assert_eq!(output.exit_code, Some(0));
    assert!(!open_backend(&repo).rebase_in_progress().unwrap());
    assert_eq!(
        git_stdout(&repo, &["rev-list", "--count", "HEAD"]),
        (before_count + 1).to_string()
    );
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%s"]),
        "fresh change"
    );
    assert_eq!(
        fs::read_to_string(repo.join("fresh.txt")).unwrap(),
        "fresh\n"
    );
}

#[test]
fn initial_multi_cherry_pick_conflict_is_reported_as_a_pause() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "file.txt", "base\n", "base");
    run_git(&repo, &["checkout", "-b", "feature"]);
    let conflicting = commit_file(&repo, "file.txt", "feature\n", "first change");
    let later = commit_file(&repo, "later.txt", "later\n", "later change");
    run_git(&repo, &["checkout", "main"]);
    commit_file(&repo, "file.txt", "main\n", "main change");

    let pick = |commit_id: String, subject: &str| InteractiveRebaseEntry {
        action: InteractiveRebaseAction::Pick,
        commit_id,
        summary: subject.to_string(),
        message: subject.to_string(),
        new_message: None,
    };
    let output = open_backend(&repo)
        .interactive_cherry_pick_with_output(&[
            pick(conflicting, "first change"),
            pick(later, "later change"),
        ])
        .expect("initial conflict should be a valid sequencer pause");

    assert_ne!(output.exit_code, Some(0));
    assert_eq!(
        open_backend(&repo).sequencer_state().unwrap(),
        SequencerState::CherryPick
    );
    assert_eq!(git_stdout(&repo, &["status", "--porcelain"]), "UU file.txt");
}

#[test]
fn cherry_pick_continue_without_resolution_is_an_error() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    fs::write(repo.join("one.txt"), "base\n").expect("write one.txt");
    commit_file(&repo, "two.txt", "base\n", "base");
    run_git(&repo, &["checkout", "-b", "feature"]);
    let first = commit_file(&repo, "one.txt", "feature one\n", "first change");
    let second = commit_file(&repo, "two.txt", "feature two\n", "second change");
    run_git(&repo, &["checkout", "main"]);
    commit_file(&repo, "one.txt", "main one\n", "main one");
    commit_file(&repo, "two.txt", "main two\n", "main two");

    let conflict = git_output(&repo, &["cherry-pick", &first, &second]);
    assert!(
        !conflict.status.success(),
        "cherry-pick should pause at the first conflict"
    );
    assert_eq!(
        open_backend(&repo).sequencer_state().unwrap(),
        SequencerState::CherryPick
    );

    // Continuing without resolving anything must fail instead of being
    // reported as a successful continue.
    open_backend(&repo)
        .rebase_continue_with_output()
        .expect_err("continue without resolution should fail");
    assert_eq!(
        open_backend(&repo).sequencer_state().unwrap(),
        SequencerState::CherryPick
    );

    // A continue that commits the resolved pick and pauses at the next
    // conflict is genuine progress and reported as a pause.
    fs::write(repo.join("one.txt"), "resolved one\n").expect("resolve first conflict");
    run_git(&repo, &["add", "one.txt"]);
    let output = open_backend(&repo)
        .rebase_continue_with_output()
        .expect("continue past the first conflict");
    assert_ne!(output.exit_code, Some(0));
    assert_eq!(
        open_backend(&repo).sequencer_state().unwrap(),
        SequencerState::CherryPick
    );
    assert_eq!(git_stdout(&repo, &["status", "--porcelain"]), "UU two.txt");

    fs::write(repo.join("two.txt"), "resolved two\n").expect("resolve second conflict");
    run_git(&repo, &["add", "two.txt"]);
    open_backend(&repo)
        .rebase_continue_with_output()
        .expect("finish the cherry-pick");
    assert_eq!(
        open_backend(&repo).sequencer_state().unwrap(),
        SequencerState::None
    );
    assert_eq!(
        git_stdout(&repo, &["log", "-2", "--format=%s"]),
        "second change\nfirst change"
    );
}

#[cfg(unix)]
#[test]
fn cherry_pick_continue_surfaces_hook_failure_on_later_step() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "file.txt", "base\n", "base");
    run_git(&repo, &["checkout", "-b", "feature"]);
    let first = commit_file(&repo, "file.txt", "feature\n", "first change");
    let second = commit_file(&repo, "second.txt", "second\n", "second change");
    run_git(&repo, &["checkout", "main"]);
    commit_file(&repo, "file.txt", "main\n", "main change");

    let pick = |commit_id: String, subject: &str| InteractiveRebaseEntry {
        action: InteractiveRebaseAction::Pick,
        commit_id,
        summary: subject.to_string(),
        message: subject.to_string(),
        new_message: None,
    };
    open_backend(&repo)
        .interactive_cherry_pick_with_output(&[
            pick(first, "first change"),
            pick(second, "second change"),
        ])
        .expect("first conflict should pause");
    install_prepare_commit_msg_hook(
        &repo,
        "#!/bin/sh\nif grep -q 'second change' \"$1\"; then\n  echo GITCOMET_PREPARE_HOOK_FAILURE >&2\n  exit 1\nfi\nexit 0\n",
    );

    fs::write(repo.join("file.txt"), "resolved\n").expect("resolve first conflict");
    run_git(&repo, &["add", "file.txt"]);
    let error = open_backend(&repo)
        .rebase_continue_with_output()
        .expect_err("later non-conflict hook failure must be surfaced");

    let message = error.to_string();
    assert!(
        message.contains("GITCOMET_PREPARE_HOOK_FAILURE"),
        "unexpected hook error: {message}"
    );
    assert_eq!(
        open_backend(&repo).sequencer_state().unwrap(),
        SequencerState::CherryPick
    );
    assert_eq!(
        git_stdout(&repo, &["diff", "--name-only", "--diff-filter=U"]),
        ""
    );
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%s"]),
        "first change"
    );
    assert_eq!(
        git_stdout(&repo, &["status", "--porcelain"]),
        "A  second.txt"
    );
}

#[test]
fn multi_cherry_pick_preserves_intentionally_empty_commits() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "base.txt", "base\n", "base");
    run_git(&repo, &["checkout", "-b", "feature"]);
    run_git(
        &repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-m",
            "empty marker",
        ],
    );
    let empty = git_stdout(&repo, &["rev-parse", "HEAD"]);
    let fresh = commit_file(&repo, "fresh.txt", "fresh\n", "fresh change");
    run_git(&repo, &["checkout", "main"]);
    let before_count: u32 = git_stdout(&repo, &["rev-list", "--count", "HEAD"])
        .parse()
        .unwrap();

    let pick = |commit_id: String, subject: &str| InteractiveRebaseEntry {
        action: InteractiveRebaseAction::Pick,
        commit_id,
        summary: subject.to_string(),
        message: subject.to_string(),
        new_message: None,
    };
    let output = open_backend(&repo)
        .interactive_cherry_pick_with_output(&[
            pick(empty, "empty marker"),
            pick(fresh, "fresh change"),
        ])
        .expect("intentionally empty commit should be preserved");

    assert_eq!(output.exit_code, Some(0));
    assert!(!open_backend(&repo).rebase_in_progress().unwrap());
    assert_eq!(
        git_stdout(&repo, &["rev-list", "--count", "HEAD"]),
        (before_count + 2).to_string()
    );
    assert_eq!(
        git_stdout(&repo, &["log", "-2", "--format=%s"]),
        "fresh change\nempty marker"
    );
}

#[test]
fn intentionally_empty_cherry_pick_signing_failure_is_not_auto_skipped() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "base.txt", "base\n", "base");
    run_git(&repo, &["checkout", "-b", "feature"]);
    run_git(
        &repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-m",
            "empty marker",
        ],
    );
    let empty = git_stdout(&repo, &["rev-parse", "HEAD"]);
    run_git(&repo, &["checkout", "main"]);
    let before_head = git_stdout(&repo, &["rev-parse", "HEAD"]);
    run_git(&repo, &["config", "commit.gpgsign", "true"]);
    run_git(&repo, &["config", "gpg.program", "false"]);

    let error = open_backend(&repo)
        .cherry_pick_with_output(&commit_id(&empty), true, None)
        .expect_err("signing failure must not be mistaken for an empty replay");

    let message = error.to_string();
    assert!(
        message.contains("sign") || message.contains("gpg"),
        "unexpected signing error: {message}"
    );
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), before_head);
    assert_eq!(git_stdout(&repo, &["status", "--porcelain"]), "");
    assert_eq!(
        open_backend(&repo).sequencer_state().unwrap(),
        SequencerState::CherryPick
    );
    assert_eq!(git_stdout(&repo, &["rev-parse", "CHERRY_PICK_HEAD"]), empty);
}

#[test]
fn intentionally_empty_merge_signing_failure_is_not_auto_skipped() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    let base = commit_file(&repo, "base.txt", "base\n", "base");
    run_git(&repo, &["checkout", "-b", "side"]);
    run_git(
        &repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-m",
            "empty side",
        ],
    );
    run_git(&repo, &["checkout", "-b", "source", &base]);
    run_git(
        &repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "merge",
            "--no-ff",
            "side",
            "-m",
            "empty merge",
        ],
    );
    let merge = git_stdout(&repo, &["rev-parse", "HEAD"]);
    run_git(&repo, &["checkout", "-b", "target", &base]);
    run_git(&repo, &["config", "commit.gpgsign", "true"]);
    run_git(&repo, &["config", "gpg.program", "false"]);

    let error = open_backend(&repo)
        .cherry_pick_with_output(&commit_id(&merge), true, Some(1))
        .expect_err("empty merge signing failure must not be auto-skipped");

    let message = error.to_string();
    assert!(
        message.contains("sign") || message.contains("gpg"),
        "unexpected signing error: {message}"
    );
    assert_eq!(git_stdout(&repo, &["rev-parse", "CHERRY_PICK_HEAD"]), merge);
    assert!(repo.join(".git/gitcomet-cherry-pick-mainline").is_file());

    open_backend(&repo)
        .rebase_abort_with_output()
        .expect("abort empty merge cherry-pick");
    assert!(!repo.join(".git/gitcomet-cherry-pick-mainline").exists());
}

#[test]
fn empty_pick_auto_skip_works_with_untracked_files_present() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "shared.txt", "old\n", "base");
    run_git(&repo, &["checkout", "-b", "feature"]);
    let applied = commit_file(&repo, "shared.txt", "new\n", "same change");
    let fresh = commit_file(&repo, "fresh.txt", "fresh\n", "fresh change");
    run_git(&repo, &["checkout", "main"]);
    commit_file(&repo, "shared.txt", "new\n", "same change independently");
    // An untracked file changes git's stop message ("nothing added to
    // commit but untracked files present"); the empty stop must still be
    // recognized from repository state.
    fs::write(repo.join("untracked.txt"), "scratch\n").expect("write untracked file");

    let pick = |commit_id: String, subject: &str| InteractiveRebaseEntry {
        action: InteractiveRebaseAction::Pick,
        commit_id,
        summary: subject.to_string(),
        message: subject.to_string(),
        new_message: None,
    };
    let output = open_backend(&repo)
        .interactive_cherry_pick_with_output(&[
            pick(applied, "same change"),
            pick(fresh, "fresh change"),
        ])
        .expect("empty pick should be skipped despite untracked files");

    assert_eq!(output.exit_code, Some(0));
    assert!(!open_backend(&repo).rebase_in_progress().unwrap());
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%s"]),
        "fresh change"
    );
    assert_eq!(
        fs::read_to_string(repo.join("untracked.txt")).unwrap(),
        "scratch\n"
    );
}

#[test]
fn multi_cherry_pick_rejects_merge_commits_before_starting() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "base.txt", "base\n", "base");
    run_git(&repo, &["checkout", "-b", "feature"]);
    let plain = commit_file(&repo, "feature.txt", "feature\n", "feature one");
    run_git(&repo, &["checkout", "-b", "side"]);
    commit_file(&repo, "side.txt", "side\n", "side change");
    run_git(&repo, &["checkout", "feature"]);
    commit_file(&repo, "feature2.txt", "feature2\n", "feature two");
    run_git(
        &repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "merge",
            "--no-ff",
            "side",
            "-m",
            "merge side",
        ],
    );
    let merge = git_stdout(&repo, &["rev-parse", "HEAD"]);
    run_git(&repo, &["checkout", "main"]);
    let before_head = git_stdout(&repo, &["rev-parse", "HEAD"]);

    let pick = |commit_id: String, subject: &str| InteractiveRebaseEntry {
        action: InteractiveRebaseAction::Pick,
        commit_id,
        summary: subject.to_string(),
        message: subject.to_string(),
        new_message: None,
    };
    // Without the up-front check, git would commit "feature one" and then
    // stop on the merge with sequencer state the UI cannot act on.
    let err = open_backend(&repo)
        .interactive_cherry_pick_with_output(&[
            pick(plain.clone(), "feature one"),
            pick(merge.clone(), "merge side"),
        ])
        .expect_err("a selected merge commit should be rejected before any pick runs");

    let message = err.to_string();
    let short_merge = merge.get(..8).unwrap_or(&merge);
    assert!(
        message.contains("multi-commit cherry-pick does not support merge commits")
            && message.contains("cherry-pick it individually and choose a mainline parent")
            && message.contains(short_merge),
        "unexpected merge rejection error: {message}"
    );
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), before_head);
    assert_eq!(git_stdout(&repo, &["status", "--porcelain"]), "");
    assert_eq!(
        open_backend(&repo).sequencer_state().unwrap(),
        SequencerState::None
    );

    // A merge the plan drops never reaches git and must not block the rest.
    let output = open_backend(&repo)
        .interactive_cherry_pick_with_output(&[
            pick(plain, "feature one"),
            InteractiveRebaseEntry {
                action: InteractiveRebaseAction::Drop,
                commit_id: merge,
                summary: "merge side".to_string(),
                message: "merge side".to_string(),
                new_message: None,
            },
        ])
        .expect("dropped merge should not block the plan");
    assert_eq!(output.exit_code, Some(0));
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%s"]),
        "feature one"
    );
}

#[test]
fn custom_cherry_pick_on_unborn_branch_reports_clear_error() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let source = dir.path().join("source");
    init_repo(&source);
    let picked = commit_file(&source, "feature.txt", "feature\n", "feature change");

    let repo = dir.path().join("repo");
    init_repo(&repo);
    run_git(&repo, &["fetch", source.to_str().expect("utf8 path")]);

    let err = open_backend(&repo)
        .interactive_cherry_pick_with_output(&[InteractiveRebaseEntry {
            action: InteractiveRebaseAction::Reword,
            commit_id: picked,
            summary: "feature change".to_string(),
            message: "feature change".to_string(),
            new_message: Some("reworded".to_string()),
        }])
        .expect_err("custom plan on unborn branch should be rejected clearly");

    let message = err.to_string();
    assert!(
        message.contains("existing commit"),
        "unexpected unborn-branch error: {message}"
    );
}

fn setup_conflicting_cherry_pick_repo(repo: &Path) -> String {
    init_repo(repo);
    commit_file(repo, "file.txt", "base\n", "base");
    run_git(repo, &["checkout", "-b", "feature"]);
    let picked = commit_file(repo, "file.txt", "feature\n", "feature change");
    run_git(repo, &["checkout", "main"]);
    picked
}

#[test]
fn conflicting_cherry_pick_returns_error_and_leaves_worktree_conflicted() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let picked = setup_conflicting_cherry_pick_repo(&repo);
    commit_file(&repo, "file.txt", "main\n", "main change");

    let err = open_backend(&repo)
        .cherry_pick(&commit_id(&picked))
        .expect_err("conflicting cherry-pick should fail");

    let message = err.to_string();
    assert!(
        message.contains("could not apply") || message.contains("CONFLICT"),
        "unexpected conflict error: {message}"
    );
    assert_eq!(git_stdout(&repo, &["status", "--porcelain"]), "UU file.txt");
    assert_eq!(
        open_backend(&repo).sequencer_state().unwrap(),
        SequencerState::CherryPick
    );
    assert!(
        git_output(&repo, &["rev-parse", "-q", "--verify", "CHERRY_PICK_HEAD"])
            .status
            .success()
    );
}

#[test]
fn dirty_worktree_rejects_cherry_pick_and_preserves_local_change() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let picked = setup_conflicting_cherry_pick_repo(&repo);
    fs::write(repo.join("file.txt"), "dirty worktree\n").expect("write dirty worktree");

    let err = open_backend(&repo)
        .cherry_pick(&commit_id(&picked))
        .expect_err("dirty worktree should reject cherry-pick");

    let message = err.to_string();
    assert!(
        message.contains("local changes") || message.contains("would be overwritten"),
        "unexpected dirty-worktree error: {message}"
    );
    assert_eq!(
        fs::read_to_string(repo.join("file.txt")).unwrap(),
        "dirty worktree\n"
    );
    assert_eq!(git_stdout(&repo, &["status", "--porcelain"]), "M file.txt");
}

#[test]
fn dirty_index_rejects_cherry_pick_and_preserves_staged_change() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let picked = setup_conflicting_cherry_pick_repo(&repo);
    fs::write(repo.join("file.txt"), "staged change\n").expect("write staged change");
    run_git(&repo, &["add", "file.txt"]);

    let err = open_backend(&repo)
        .cherry_pick(&commit_id(&picked))
        .expect_err("dirty index should reject cherry-pick");

    let message = err.to_string();
    assert!(
        message.contains("local changes") || message.contains("would be overwritten"),
        "unexpected dirty-index error: {message}"
    );
    assert_eq!(
        fs::read_to_string(repo.join("file.txt")).unwrap(),
        "staged change\n"
    );
    assert_eq!(git_stdout(&repo, &["status", "--porcelain"]), "M  file.txt");
}

#[test]
fn continue_falls_back_to_cherry_pick_continue_when_cherry_pick_is_paused() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "file.txt", "base\n", "base");

    run_git(&repo, &["checkout", "-b", "feature"]);
    let picked = commit_file(&repo, "file.txt", "feature\n", "feature change");

    run_git(&repo, &["checkout", "main"]);
    commit_file(&repo, "file.txt", "main\n", "main change");

    let conflict = git_output(&repo, &["cherry-pick", &picked]);
    assert!(
        !conflict.status.success(),
        "cherry-pick should pause at a conflict"
    );
    assert!(open_backend(&repo).rebase_in_progress().unwrap());
    assert_eq!(
        open_backend(&repo).sequencer_state().unwrap(),
        SequencerState::CherryPick
    );

    fs::write(repo.join("file.txt"), "resolved\n").expect("resolve conflict");
    run_git(&repo, &["add", "file.txt"]);

    let output = open_backend(&repo)
        .rebase_continue_with_output()
        .expect("continue paused cherry-pick");
    assert_eq!(output.command, "git cherry-pick --continue");
    assert!(!open_backend(&repo).rebase_in_progress().unwrap());
    assert!(
        !git_output(&repo, &["rev-parse", "-q", "--verify", "CHERRY_PICK_HEAD"])
            .status
            .success()
    );
    assert_eq!(
        fs::read_to_string(repo.join("file.txt")).unwrap(),
        "resolved\n"
    );
}

#[test]
fn abort_returns_active_cherry_pick_lock_error() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let picked = setup_conflicting_cherry_pick_repo(&repo);
    commit_file(&repo, "file.txt", "main\n", "main change");
    let conflict = git_output(&repo, &["cherry-pick", &picked]);
    assert!(!conflict.status.success(), "cherry-pick should conflict");
    fs::write(repo.join(".git").join("index.lock"), "locked\n").expect("create index lock");

    let error = open_backend(&repo)
        .rebase_abort_with_output()
        .expect_err("active cherry-pick abort error should be returned");

    let message = error.to_string();
    assert!(
        message.contains("index.lock") || message.contains("Unable to create"),
        "unexpected abort error: {message}"
    );
    assert!(
        !message.contains("No rebase in progress"),
        "cherry-pick error was replaced by the rebase fallback: {message}"
    );
    assert_eq!(
        open_backend(&repo).sequencer_state().unwrap(),
        SequencerState::CherryPick
    );
}
<<<<<<< New base: Support explicit commit ranges when cherry-picking onto a new branch (#17)

#[test]
fn cherry_pick_range_onto_new_branch_creates_branch_and_applies_source_commits_in_order() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    let base = commit_file(&repo, "base.txt", "base\n", "base");
    // D: the base branch C starts from (has its own commit).
    run_git(&repo, &["checkout", "-b", "branch_d", &base]);
    commit_file(&repo, "d.txt", "d\n", "d work");
    // A: the source branch whose commits are copied; B (range) = the base
    // commit, an ancestor of A.
    run_git(&repo, &["checkout", "-b", "branch_a", &base]);
    commit_file(&repo, "a1.txt", "a1\n", "a one");
    commit_file(&repo, "a2.txt", "a2\n", "a two");
    // The caller sits on some other branch; the command must move to branch_c.
    run_git(&repo, &["checkout", "branch_d"]);

    open_backend(&repo)
        .cherry_pick_range_onto_new_branch("branch_d", &base, "branch_a", "branch_c")
        .expect("cherry-pick range onto new branch");

    assert_eq!(git_stdout(&repo, &["branch", "--show-current"]), "branch_c");
    assert_eq!(
        git_stdout(&repo, &["log", "--format=%s"]),
        "a two\na one\nd work\nbase"
    );
    assert_eq!(
        fs::read_to_string(repo.join("a1.txt")).unwrap(),
        "a1\n"
    );
    assert_eq!(
        fs::read_to_string(repo.join("a2.txt")).unwrap(),
        "a2\n"
    );
}

#[test]
fn cherry_pick_range_onto_new_branch_skips_merge_commits() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    let base = commit_file(&repo, "base.txt", "base\n", "base");
    // D: the base branch C starts from.
    run_git(&repo, &["checkout", "-b", "branch_d", &base]);
    commit_file(&repo, "d.txt", "d\n", "d work");
    // A: the source branch merged into main below.
    run_git(&repo, &["checkout", "-b", "branch_a", &base]);
    commit_file(&repo, "a1.txt", "a1\n", "a one");
    commit_file(&repo, "a2.txt", "a2\n", "a two");
    // main grows a commit and then merges branch_a in, producing a merge
    // commit inside the range (B = the base commit, an ancestor of main)
    // that must be skipped.
    run_git(&repo, &["checkout", "main"]);
    commit_file(&repo, "main.txt", "main\n", "main change");
    run_git(&repo, &["merge", "--no-edit", "branch_a"]);

    open_backend(&repo)
        .cherry_pick_range_onto_new_branch("branch_d", &base, "main", "branch_c")
        .expect("cherry-pick range onto new branch");

    assert_eq!(git_stdout(&repo, &["branch", "--show-current"]), "branch_c");
    assert_eq!(
        git_stdout(&repo, &["log", "--format=%s"]),
        "main change\na two\na one\nd work\nbase"
    );
}

#[test]
fn cherry_pick_range_onto_new_branch_rejects_existing_branch_and_empty_range() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    let base = commit_file(&repo, "base.txt", "base\n", "base");
    run_git(&repo, &["checkout", "-b", "branch_d", &base]);
    commit_file(&repo, "d.txt", "d\n", "d work");
    run_git(&repo, &["checkout", "-b", "branch_a", &base]);
    commit_file(&repo, "a1.txt", "a1\n", "a one");
    run_git(&repo, &["checkout", "-b", "branch_c", "branch_d"]);

    // Branch C already exists: nothing may change.
    let error = open_backend(&repo)
        .cherry_pick_range_onto_new_branch("branch_d", &base, "branch_a", "branch_c")
        .expect_err("existing branch must be rejected");
    assert!(error.to_string().contains("already exists"), "{error}");
    assert_eq!(git_stdout(&repo, &["branch", "--show-current"]), "branch_c");

    // Empty range (source == range): nothing created.
    run_git(&repo, &["checkout", "branch_d"]);
    let error = open_backend(&repo)
        .cherry_pick_range_onto_new_branch("branch_d", "branch_a", "branch_a", "branch_e")
        .expect_err("empty range must be rejected");
    assert!(error.to_string().contains("no commits"), "{error}");
    assert_eq!(git_stdout(&repo, &["branch", "--list", "branch_e"]), "");
    assert_eq!(git_stdout(&repo, &["branch", "--show-current"]), "branch_d");
}

#[test]
fn cherry_pick_range_onto_new_branch_rejects_non_ancestor_range() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    let base = commit_file(&repo, "base.txt", "base\n", "base");
    run_git(&repo, &["checkout", "-b", "branch_d", &base]);
    commit_file(&repo, "d.txt", "d\n", "d work");
    run_git(&repo, &["checkout", "-b", "branch_a", &base]);
    commit_file(&repo, "a1.txt", "a1\n", "a one");

    // B = branch_d is not an ancestor of A = branch_a (they diverge from
    // base): rejected before anything is created.
    let error = open_backend(&repo)
        .cherry_pick_range_onto_new_branch("branch_d", "branch_d", "branch_a", "branch_c")
        .expect_err("non-ancestor range must be rejected");
    assert!(error.to_string().contains("ancestor"), "{error}");
    assert_eq!(git_stdout(&repo, &["branch", "--list", "branch_c"]), "");
    assert_eq!(git_stdout(&repo, &["branch", "--show-current"]), "branch_a");
}
||||||| Common ancestor
=======

#[test]
fn cherry_pick_range_onto_new_branch_creates_branch_and_applies_source_commits_in_order() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    let base = commit_file(&repo, "base.txt", "base\n", "base");
    run_git(&repo, &["checkout", "-b", "branch_b", &base]);
    commit_file(&repo, "b.txt", "b\n", "b work");
    run_git(&repo, &["checkout", "-b", "branch_a", &base]);
    commit_file(&repo, "a1.txt", "a1\n", "a one");
    commit_file(&repo, "a2.txt", "a2\n", "a two");
    // The caller sits on some other branch; the command must move to branch_c.
    run_git(&repo, &["checkout", "branch_b"]);

    open_backend(&repo)
        .cherry_pick_range_onto_new_branch("branch_b", "branch_a", "branch_c")
        .expect("cherry-pick range onto new branch");

    assert_eq!(git_stdout(&repo, &["branch", "--show-current"]), "branch_c");
    assert_eq!(
        git_stdout(&repo, &["log", "--format=%s"]),
        "a two\na one\nb work\nbase"
    );
    assert_eq!(
        fs::read_to_string(repo.join("a1.txt")).unwrap(),
        "a1\n"
    );
    assert_eq!(
        fs::read_to_string(repo.join("a2.txt")).unwrap(),
        "a2\n"
    );
}

#[test]
fn cherry_pick_range_onto_new_branch_skips_merge_commits() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    let base = commit_file(&repo, "base.txt", "base\n", "base");
    run_git(&repo, &["checkout", "-b", "branch_b", &base]);
    commit_file(&repo, "b.txt", "b\n", "b work");
    run_git(&repo, &["checkout", "-b", "branch_a", &base]);
    commit_file(&repo, "a1.txt", "a1\n", "a one");
    commit_file(&repo, "a2.txt", "a2\n", "a two");
    // main grows a commit and then merges branch_a in, producing a merge
    // commit inside the branch_b..main range that must be skipped.
    run_git(&repo, &["checkout", "main"]);
    commit_file(&repo, "main.txt", "main\n", "main change");
    run_git(&repo, &["merge", "--no-edit", "branch_a"]);

    open_backend(&repo)
        .cherry_pick_range_onto_new_branch("branch_b", "main", "branch_c")
        .expect("cherry-pick range onto new branch");

    assert_eq!(git_stdout(&repo, &["branch", "--show-current"]), "branch_c");
    assert_eq!(
        git_stdout(&repo, &["log", "--format=%s"]),
        "main change\na two\na one\nb work\nbase"
    );
}

#[test]
fn cherry_pick_range_onto_new_branch_rejects_existing_branch_and_empty_range() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    let base = commit_file(&repo, "base.txt", "base\n", "base");
    run_git(&repo, &["checkout", "-b", "branch_b", &base]);
    commit_file(&repo, "b.txt", "b\n", "b work");
    run_git(&repo, &["checkout", "-b", "branch_a", &base]);
    commit_file(&repo, "a1.txt", "a1\n", "a one");
    run_git(&repo, &["checkout", "-b", "branch_c", "branch_b"]);

    // Branch C already exists: nothing may change.
    let error = open_backend(&repo)
        .cherry_pick_range_onto_new_branch("branch_b", "branch_a", "branch_c")
        .expect_err("existing branch must be rejected");
    assert!(error.to_string().contains("already exists"), "{error}");
    assert_eq!(git_stdout(&repo, &["branch", "--show-current"]), "branch_c");

    // Empty range (branch_a is fully contained in branch_b): nothing created.
    run_git(&repo, &["checkout", "branch_b"]);
    let error = open_backend(&repo)
        .cherry_pick_range_onto_new_branch("branch_b", "branch_b", "branch_d")
        .expect_err("empty range must be rejected");
    assert!(error.to_string().contains("no commits"), "{error}");
    assert_eq!(
        git_stdout(&repo, &["branch", "--list", "branch_d"]),
        ""
    );
    assert_eq!(git_stdout(&repo, &["branch", "--show-current"]), "branch_b");
}
>>>>>>> Current commit: Add cherry-pick branch A onto B as new branch C from the action bar
