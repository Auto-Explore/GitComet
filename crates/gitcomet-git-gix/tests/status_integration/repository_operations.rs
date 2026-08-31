use super::*;

#[test]
fn stage_and_unstage_paths_update_status() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    write(repo, "a.txt", "one\ntwo\n");
    write(repo, "b.txt", "untracked\n");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened.stage(&[Path::new("a.txt")]).unwrap();
    let status = opened.status().unwrap();
    assert_eq!(status.staged.len(), 1);
    assert_eq!(status.staged[0].path, PathBuf::from("a.txt"));
    assert_eq!(status.staged[0].kind, FileStatusKind::Modified);
    assert_eq!(status.unstaged.len(), 1);
    assert_eq!(status.unstaged[0].path, PathBuf::from("b.txt"));
    assert_eq!(status.unstaged[0].kind, FileStatusKind::Untracked);

    opened.unstage(&[Path::new("a.txt")]).unwrap();
    let status = opened.status().unwrap();
    assert!(status.staged.is_empty());
    assert_eq!(status.unstaged.len(), 2);
    assert!(
        status
            .unstaged
            .iter()
            .any(|e| e.path == Path::new("a.txt") && e.kind == FileStatusKind::Modified)
    );
    assert!(
        status
            .unstaged
            .iter()
            .any(|e| e.path == Path::new("b.txt") && e.kind == FileStatusKind::Untracked)
    );
}

#[test]
fn unstage_empty_paths_with_head_unstages_all_index_changes() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    write(repo, "b.txt", "base\n");
    run_git(repo, &["add", "a.txt", "b.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    write(repo, "a.txt", "one\ntwo\n");
    write(repo, "b.txt", "base\nnext\n");
    run_git(repo, &["add", "a.txt", "b.txt"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened.unstage(&[]).unwrap();

    let staged = run_git_output(repo, &["diff", "--cached", "--name-only"]);
    assert!(
        staged.is_empty(),
        "expected empty staged diff, got {staged:?}"
    );

    let unstaged = run_git_output(repo, &["diff", "--name-only"]);
    assert!(
        unstaged.lines().any(|line| line == "a.txt"),
        "expected a.txt to be unstaged-modified, got {unstaged:?}"
    );
    assert!(
        unstaged.lines().any(|line| line == "b.txt"),
        "expected b.txt to be unstaged-modified, got {unstaged:?}"
    );
}

#[test]
fn unstage_empty_paths_without_head_unstages_all_added_paths() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);

    write(repo, "a.txt", "one\n");
    write(repo, "b.txt", "two\n");
    run_git(repo, &["add", "a.txt", "b.txt"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened.unstage(&[]).unwrap();

    let staged = run_git_output(repo, &["diff", "--cached", "--name-only"]);
    assert!(
        staged.is_empty(),
        "expected empty staged diff, got {staged:?}"
    );

    let short = run_git_output(repo, &["status", "--short"]);
    assert!(
        short.lines().any(|line| line == "?? a.txt"),
        "expected a.txt to be untracked after unstage-all, got {short:?}"
    );
    assert!(
        short.lines().any(|line| line == "?? b.txt"),
        "expected b.txt to be untracked after unstage-all, got {short:?}"
    );
}

#[test]
fn unstage_paths_without_head_only_unstages_selected_entries() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);

    write(repo, "a.txt", "one\n");
    write(repo, "b.txt", "two\n");
    run_git(repo, &["add", "a.txt", "b.txt"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened.unstage(&[Path::new("a.txt")]).unwrap();

    let short = run_git_output(repo, &["status", "--short"]);
    assert!(
        short.lines().any(|line| line == "?? a.txt"),
        "expected a.txt to be untracked after targeted unstage, got {short:?}"
    );
    assert!(
        short.lines().any(|line| line == "A  b.txt"),
        "expected b.txt to remain staged after targeted unstage, got {short:?}"
    );
}

#[test]
fn commit_creates_new_commit_and_cleans_status() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    write(repo, "a.txt", "one\ntwo\n");
    run_git(repo, &["add", "a.txt"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened.commit("second").unwrap();

    let msg = git_command()
        .arg("-C")
        .arg(repo)
        .args(["log", "-1", "--pretty=%B"])
        .output()
        .expect("git log to run");
    assert!(msg.status.success());
    assert_eq!(String::from_utf8(msg.stdout).unwrap().trim(), "second");

    let status = opened.status().unwrap();
    assert!(status.staged.is_empty());
    assert!(status.unstaged.is_empty());
}

#[test]
fn reset_soft_moves_head_and_leaves_changes_staged() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "c1"]);
    let c1 = git_command()
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse c1");
    assert!(c1.status.success());
    let c1 = String::from_utf8(c1.stdout).unwrap().trim().to_string();

    write(repo, "a.txt", "two\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "c2"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened
        .reset_with_output("HEAD~1", gitcomet_core::services::ResetMode::Soft)
        .unwrap();

    let head = git_command()
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse head");
    assert!(head.status.success());
    assert_eq!(String::from_utf8(head.stdout).unwrap().trim(), c1);
    assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "two\n");

    let status = opened.status().unwrap();
    assert_eq!(status.staged.len(), 1);
    assert_eq!(status.staged[0].path, PathBuf::from("a.txt"));
    assert_eq!(status.staged[0].kind, FileStatusKind::Modified);
    assert!(status.unstaged.is_empty());
}

#[test]
fn reset_mixed_moves_head_and_leaves_changes_unstaged() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "c1"]);
    let c1 = git_command()
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse c1");
    assert!(c1.status.success());
    let c1 = String::from_utf8(c1.stdout).unwrap().trim().to_string();

    write(repo, "a.txt", "two\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "c2"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened
        .reset_with_output("HEAD~1", gitcomet_core::services::ResetMode::Mixed)
        .unwrap();

    let head = git_command()
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse head");
    assert!(head.status.success());
    assert_eq!(String::from_utf8(head.stdout).unwrap().trim(), c1);
    assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "two\n");

    let status = opened.status().unwrap();
    assert!(status.staged.is_empty());
    assert_eq!(status.unstaged.len(), 1);
    assert_eq!(status.unstaged[0].path, PathBuf::from("a.txt"));
    assert_eq!(status.unstaged[0].kind, FileStatusKind::Modified);
}

#[test]
fn reset_hard_moves_head_and_discards_changes() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "c1"]);
    let c1 = git_command()
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse c1");
    assert!(c1.status.success());
    let c1 = String::from_utf8(c1.stdout).unwrap().trim().to_string();

    write(repo, "a.txt", "two\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "c2"]);

    write(repo, "a.txt", "two-modified\n");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened
        .reset_with_output("HEAD~1", gitcomet_core::services::ResetMode::Hard)
        .unwrap();

    let head = git_command()
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse head");
    assert!(head.status.success());
    assert_eq!(String::from_utf8(head.stdout).unwrap().trim(), c1);
    assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "one\n");

    let status = opened.status().unwrap();
    assert!(status.staged.is_empty());
    assert!(status.unstaged.is_empty());
}

#[test]
fn revert_commit_creates_new_commit_and_reverts_content() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "c1"]);

    write(repo, "a.txt", "two\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "c2"]);

    let c2 = git_command()
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse c2");
    assert!(c2.status.success());
    let c2 = String::from_utf8(c2.stdout).unwrap().trim().to_string();

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened
        .revert(&gitcomet_core::domain::CommitId(c2.clone().into()))
        .unwrap();

    assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "one\n");
    let status = opened.status().unwrap();
    assert!(status.staged.is_empty());
    assert!(status.unstaged.is_empty());

    let head = git_command()
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse head");
    assert!(head.status.success());
    let head = String::from_utf8(head.stdout).unwrap().trim().to_string();
    assert_ne!(head, c2, "expected revert to create a new commit");
}

#[test]
fn amend_rewrites_head_commit_message_and_content() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    let head_before = git_command()
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse head");
    assert!(head_before.status.success());
    let head_before = String::from_utf8(head_before.stdout)
        .unwrap()
        .trim()
        .to_string();

    write(repo, "a.txt", "one\ntwo\n");
    run_git(repo, &["add", "a.txt"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened.commit_amend("amended").unwrap();

    let head_after = git_command()
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse head");
    assert!(head_after.status.success());
    let head_after = String::from_utf8(head_after.stdout)
        .unwrap()
        .trim()
        .to_string();
    assert_ne!(head_after, head_before);

    let count = git_command()
        .arg("-C")
        .arg(repo)
        .args(["rev-list", "--count", "HEAD"])
        .output()
        .expect("rev-list --count");
    assert!(count.status.success());
    assert_eq!(String::from_utf8(count.stdout).unwrap().trim(), "1");

    let msg = git_command()
        .arg("-C")
        .arg(repo)
        .args(["log", "-1", "--pretty=%B"])
        .output()
        .expect("git log to run");
    assert!(msg.status.success());
    assert_eq!(String::from_utf8(msg.stdout).unwrap().trim(), "amended");
    assert_eq!(
        fs::read_to_string(repo.join("a.txt")).unwrap(),
        "one\ntwo\n"
    );

    let status = opened.status().unwrap();
    assert!(status.staged.is_empty());
    assert!(status.unstaged.is_empty());
}

#[test]
fn merge_creates_merge_commit_when_branches_diverged() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "b.txt", "feature\n");
    run_git(repo, &["add", "b.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );

    run_git(repo, &["checkout", "-"]);
    write(repo, "c.txt", "main\n");
    run_git(repo, &["add", "c.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "main"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened.merge_ref_with_output("feature").unwrap();

    let parents = git_command()
        .arg("-C")
        .arg(repo)
        .args(["rev-list", "--parents", "-n", "1", "HEAD"])
        .output()
        .expect("rev-list --parents");
    assert!(parents.status.success());
    let parent_count = String::from_utf8(parents.stdout)
        .unwrap()
        .split_whitespace()
        .count()
        .saturating_sub(1);
    assert_eq!(parent_count, 2, "expected merge commit");

    assert!(repo.join("b.txt").exists());
    assert!(repo.join("c.txt").exists());
    assert_eq!(fs::read_to_string(repo.join("b.txt")).unwrap(), "feature\n");
    assert_eq!(fs::read_to_string(repo.join("c.txt")).unwrap(), "main\n");
}

#[test]
fn merge_fast_forwards_when_possible_even_if_merge_ff_is_disabled() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "b.txt", "feature\n");
    run_git(repo, &["add", "b.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );

    run_git(repo, &["checkout", "-"]);
    run_git(repo, &["config", "merge.ff", "false"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened.merge_ref_with_output("feature").unwrap();

    let parents = git_command()
        .arg("-C")
        .arg(repo)
        .args(["rev-list", "--parents", "-n", "1", "HEAD"])
        .output()
        .expect("rev-list --parents");
    assert!(parents.status.success());
    let parent_count = String::from_utf8(parents.stdout)
        .unwrap()
        .split_whitespace()
        .count()
        .saturating_sub(1);
    assert_eq!(parent_count, 1, "expected fast-forward");

    let msg = git_command()
        .arg("-C")
        .arg(repo)
        .args(["log", "-1", "--pretty=%B"])
        .output()
        .expect("git log to run");
    assert!(msg.status.success());
    assert_eq!(String::from_utf8(msg.stdout).unwrap().trim(), "feature");
}

#[test]
fn squash_ref_stages_changes_without_creating_merge_commit() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "b.txt", "feature\n");
    run_git(repo, &["add", "b.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );

    run_git(repo, &["checkout", "-"]);
    write(repo, "c.txt", "main\n");
    run_git(repo, &["add", "c.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "main"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let output = opened
        .squash_ref_with_output("feature")
        .expect("squash should succeed");
    assert_eq!(output.exit_code, Some(0));

    let parents = git_command()
        .arg("-C")
        .arg(repo)
        .args(["rev-list", "--parents", "-n", "1", "HEAD"])
        .output()
        .expect("rev-list --parents");
    assert!(parents.status.success());
    let parent_count = String::from_utf8(parents.stdout)
        .unwrap()
        .split_whitespace()
        .count()
        .saturating_sub(1);
    assert_eq!(parent_count, 1, "squash should not create a merge commit");

    assert_eq!(fs::read_to_string(repo.join("b.txt")).unwrap(), "feature\n");

    let status = opened.status().unwrap();
    assert!(
        status
            .staged
            .iter()
            .any(|f| f.path.as_path() == Path::new("b.txt")),
        "expected squashed changes to be staged"
    );
}

#[test]
fn merge_commit_message_is_available_during_conflict() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "a.txt", "feature\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );

    run_git(repo, &["checkout", "-"]);
    write(repo, "a.txt", "main\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "main"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    assert!(opened.merge_ref_with_output("feature").is_err());

    let msg = opened
        .merge_commit_message()
        .unwrap()
        .expect("merge commit message");
    assert_eq!(
        msg.lines().next().unwrap_or_default(),
        "Merge branch 'feature'"
    );
    assert!(
        !msg.contains('#'),
        "expected message to be cleaned, got: {msg}"
    );

    run_git(repo, &["merge", "--abort"]);
    assert!(opened.merge_commit_message().unwrap().is_none());
}

#[test]
fn commit_finishes_merge_when_resolved_tree_matches_head() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "a.txt", "feature\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );

    run_git(repo, &["checkout", "-"]);
    write(repo, "a.txt", "main\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "main"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    assert!(opened.merge_ref_with_output("feature").is_err());
    run_git(repo, &["checkout", "--ours", "a.txt"]);
    run_git(repo, &["add", "a.txt"]);

    let status = opened.status().unwrap();
    assert!(status.staged.is_empty(), "expected no staged changes");
    assert!(status.unstaged.is_empty(), "expected no unstaged changes");

    opened
        .commit("Merge branch 'feature'")
        .expect("merge commit should succeed even without tree changes");

    assert!(opened.merge_commit_message().unwrap().is_none());

    let parents = git_command()
        .arg("-C")
        .arg(repo)
        .args(["rev-list", "--parents", "-n", "1", "HEAD"])
        .output()
        .expect("rev-list --parents");
    assert!(parents.status.success());
    let parent_count = String::from_utf8(parents.stdout)
        .unwrap()
        .split_whitespace()
        .count()
        .saturating_sub(1);
    assert_eq!(parent_count, 2, "expected merge commit");
}

#[test]
fn rebase_replays_commits_onto_target_branch() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "b.txt", "feature\n");
    run_git(repo, &["add", "b.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );

    run_git(repo, &["checkout", "-"]);
    write(repo, "c.txt", "main\n");
    run_git(repo, &["add", "c.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "main"],
    );
    let master_head = git_command()
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse master");
    assert!(master_head.status.success());
    let master_head = String::from_utf8(master_head.stdout)
        .unwrap()
        .trim()
        .to_string();

    run_git(repo, &["checkout", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened.rebase_with_output("main").unwrap();

    let parent = git_command()
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD^"])
        .output()
        .expect("rev-parse parent");
    assert!(parent.status.success());
    assert_eq!(
        String::from_utf8(parent.stdout).unwrap().trim(),
        master_head
    );

    assert!(repo.join("b.txt").exists());
    assert_eq!(fs::read_to_string(repo.join("b.txt")).unwrap(), "feature\n");
    let status = opened.status().unwrap();
    assert!(status.staged.is_empty());
    assert!(status.unstaged.is_empty());
}

#[test]
fn rebase_replays_commits_onto_target_sha() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["commit", "-m", "base"]);

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "b.txt", "feature\n");
    run_git(repo, &["add", "b.txt"]);
    run_git(repo, &["commit", "-m", "feature"]);

    run_git(repo, &["checkout", "main"]);
    write(repo, "c.txt", "main\n");
    run_git(repo, &["add", "c.txt"]);
    run_git(repo, &["commit", "-m", "main"]);
    let target_sha = run_git_output(repo, &["rev-parse", "HEAD"]);

    run_git(repo, &["checkout", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened.rebase_with_output(&target_sha).unwrap();

    assert_eq!(run_git_output(repo, &["rev-parse", "HEAD^"]), target_sha);
    assert_eq!(fs::read_to_string(repo.join("b.txt")).unwrap(), "feature\n");
    let status = opened.status().unwrap();
    assert!(status.staged.is_empty());
    assert!(status.unstaged.is_empty());
}

#[test]
fn rebase_in_progress_and_abort_round_trip() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "a.txt", "feature\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );

    run_git(repo, &["checkout", "main"]);
    write(repo, "a.txt", "main\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "main"],
    );

    run_git(repo, &["checkout", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    assert!(!opened.rebase_in_progress().unwrap());
    assert!(opened.rebase_with_output("main").is_err());
    assert!(opened.rebase_in_progress().unwrap());

    opened.rebase_abort_with_output().unwrap();
    assert!(!opened.rebase_in_progress().unwrap());
}

#[test]
fn rebase_continue_without_in_progress_rebase_returns_error() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    assert!(opened.rebase_continue_with_output().is_err());
}

#[test]
fn rebase_continue_paused_at_next_conflict_is_ok() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "f.txt", "v0\n");
    run_git(repo, &["add", "f.txt"]);
    run_git(repo, &["commit", "-m", "base"]);
    let default_branch = run_git_output(repo, &["rev-parse", "--abbrev-ref", "HEAD"])
        .trim()
        .to_string();

    // Two feature commits, each of which will conflict when rebased onto a
    // divergent `onto` commit.
    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "f.txt", "A\n");
    run_git(repo, &["commit", "-am", "A"]);
    write(repo, "f.txt", "B\n");
    run_git(repo, &["commit", "-am", "B"]);

    run_git(repo, &["checkout", &default_branch]);
    write(repo, "f.txt", "onto\n");
    run_git(repo, &["commit", "-am", "onto"]);

    // Start rebasing `feature` onto the divergent branch: pauses at A's conflict.
    run_git(repo, &["checkout", "feature"]);
    run_git_expect_failure(repo, &["rebase", &default_branch]);

    // Resolve the first conflict.
    write(repo, "f.txt", "resolved-A\n");
    run_git(repo, &["add", "f.txt"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    // Continuing applies B, which conflicts again. This pauses the rebase at the
    // next conflict — a normal outcome, not a failure — so it must be Ok and the
    // rebase must still be in progress (regression test for the stuck-spinner bug).
    let result = opened.rebase_continue_with_output();
    assert!(
        result.is_ok(),
        "rebase --continue that pauses at the next conflict should be Ok, got {result:?}"
    );
    assert!(
        opened.rebase_in_progress().unwrap(),
        "rebase should still be in progress after pausing at the next conflict"
    );
}

#[test]
fn rebase_abort_falls_back_to_git_am_abort() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "a.txt", "feature\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );

    let patch_output = git_command()
        .arg("-C")
        .arg(repo)
        .args(["format-patch", "-1", "HEAD", "--stdout"])
        .output()
        .expect("git format-patch to run");
    assert!(
        patch_output.status.success(),
        "git format-patch failed: {}",
        String::from_utf8_lossy(&patch_output.stderr)
    );

    let patch_file = tempfile::NamedTempFile::new().expect("create patch temp file");
    fs::write(patch_file.path(), &patch_output.stdout).expect("write patch file");

    run_git(repo, &["checkout", "main"]);
    write(repo, "a.txt", "main\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "main"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    assert!(opened.apply_patch_with_output(patch_file.path()).is_err());
    assert!(
        opened.rebase_in_progress().unwrap(),
        "expected apply-patch sequencer state to be in progress"
    );

    let abort_output = opened.rebase_abort_with_output().unwrap();
    assert_eq!(
        abort_output.command, "git am --abort",
        "expected rebase abort fallback to use git am --abort"
    );
    assert!(!opened.rebase_in_progress().unwrap());

    let status = opened.status().unwrap();
    assert!(status.staged.is_empty());
    assert!(status.unstaged.is_empty());
    assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "main\n");
}

#[test]
fn merge_abort_with_output_clears_conflict_state() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "a.txt", "feature\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );

    run_git(repo, &["checkout", "-"]);
    write(repo, "a.txt", "main\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "main"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    assert!(opened.merge_ref_with_output("feature").is_err());
    assert!(opened.merge_commit_message().unwrap().is_some());

    opened.merge_abort_with_output().unwrap();

    assert!(opened.merge_commit_message().unwrap().is_none());
    let status = opened.status().unwrap();
    assert!(status.staged.is_empty());
    assert!(status.unstaged.is_empty());
}

#[test]
fn create_rename_and_delete_local_branch() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let head = git_command()
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse HEAD");
    assert!(head.status.success());
    let head = String::from_utf8(head.stdout)
        .expect("HEAD is utf-8")
        .trim()
        .to_owned();

    opened
        .create_branch("feature", &gitcomet_core::domain::CommitId(head.into()))
        .unwrap();
    run_git(
        repo,
        &["show-ref", "--verify", "--quiet", "refs/heads/feature"],
    );

    opened.rename_branch("feature", "renamed-feature").unwrap();
    run_git(
        repo,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            "refs/heads/renamed-feature",
        ],
    );
    let old_name = git_command()
        .arg("-C")
        .arg(repo)
        .args(["show-ref", "--verify", "--quiet", "refs/heads/feature"])
        .status()
        .expect("show-ref old branch name");
    assert!(
        !old_name.success(),
        "expected old branch name to be removed"
    );

    opened.delete_branch("renamed-feature").unwrap();
    let deleted = git_command()
        .arg("-C")
        .arg(repo)
        .args([
            "show-ref",
            "--verify",
            "--quiet",
            "refs/heads/renamed-feature",
        ])
        .status()
        .expect("show-ref");
    assert!(!deleted.success(), "expected branch to be deleted");
}

#[test]
fn create_branch_existing_branch_returns_structured_git_error() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    let head = run_git_output(repo, &["rev-parse", "HEAD"]);
    run_git(repo, &["branch", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let err = opened
        .create_branch("feature", &gitcomet_core::domain::CommitId(head.into()))
        .expect_err("creating an existing branch should fail");
    assert_git_failure(&err, "git branch", GitFailureId::CommandFailed);
    let ErrorKind::Git(failure) = err.kind() else {
        unreachable!();
    };
    assert_eq!(failure.exit_code(), Some(128));
    assert_eq!(
        failure.detail(),
        Some("fatal: a branch named 'feature' already exists")
    );
}

#[test]
fn create_branch_on_unborn_head_returns_structured_git_error() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let err = opened
        .create_branch("feature", &gitcomet_core::domain::CommitId("HEAD".into()))
        .expect_err("creating a branch on unborn HEAD should fail");
    assert_git_failure(&err, "git branch", GitFailureId::CommandFailed);
    let ErrorKind::Git(failure) = err.kind() else {
        unreachable!();
    };
    assert_eq!(failure.exit_code(), Some(128));
    assert_eq!(
        failure.detail(),
        Some("fatal: not a valid object name: 'HEAD'")
    );

    let exists = git_command()
        .arg("-C")
        .arg(repo)
        .args(["show-ref", "--verify", "--quiet", "refs/heads/feature"])
        .status()
        .expect("show-ref feature");
    assert!(!exists.success(), "feature branch should not be created");
}

#[test]
fn create_branch_from_detached_head_using_head_revision() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "first"],
    );

    write(repo, "a.txt", "two\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "second"],
    );

    let first_commit = run_git_output(repo, &["rev-parse", "HEAD~1"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    opened
        .checkout_commit(&gitcomet_core::domain::CommitId(
            first_commit.clone().into(),
        ))
        .unwrap();
    opened
        .create_branch("rescue", &gitcomet_core::domain::CommitId("HEAD".into()))
        .unwrap();

    let rescue_target = run_git_output(repo, &["rev-parse", "rescue"]);
    assert_eq!(rescue_target, first_commit);
}

#[test]
fn create_branch_from_annotated_tag_peels_to_commit() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
    run_git(repo, &["config", "tag.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    let head = run_git_output(repo, &["rev-parse", "HEAD"]);
    run_git(repo, &["tag", "-a", "v1", "-m", "v1"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    opened
        .create_branch("feature", &gitcomet_core::domain::CommitId("v1".into()))
        .unwrap();

    let feature_target = run_git_output(repo, &["rev-parse", "feature"]);
    assert_eq!(feature_target, head);
    let feature_kind = run_git_output(repo, &["cat-file", "-t", "feature"]);
    assert_eq!(feature_kind, "commit");
}

#[test]
fn create_branch_from_blob_target_returns_structured_git_error() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    let blob = hash_blob(repo, b"blob target\n");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let err = opened
        .create_branch(
            "feature",
            &gitcomet_core::domain::CommitId(blob.clone().into()),
        )
        .expect_err("creating a branch from a blob should fail");
    assert_git_failure(&err, "git branch", GitFailureId::CommandFailed);
    let ErrorKind::Git(failure) = err.kind() else {
        unreachable!();
    };
    assert_eq!(failure.exit_code(), Some(128));
    let detail = failure.detail().expect("git detail");
    assert!(
        detail.contains("not a valid branch point"),
        "unexpected create-branch detail: {detail}"
    );
    assert!(
        detail.contains(&blob),
        "expected blob id in create-branch detail: {detail}"
    );
}

#[test]
fn create_branch_head_target_reflects_move_after_backend_open() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "first"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    write(repo, "a.txt", "two\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "second"],
    );
    let second_commit = run_git_output(repo, &["rev-parse", "HEAD"]);

    opened
        .create_branch("feature", &gitcomet_core::domain::CommitId("HEAD".into()))
        .unwrap();

    let feature_target = run_git_output(repo, &["rev-parse", "feature"]);
    assert_eq!(feature_target, second_commit);
}

#[test]
fn create_branch_target_branch_created_after_backend_open() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    let head = run_git_output(repo, &["rev-parse", "HEAD"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    run_git(repo, &["branch", "source"]);

    opened
        .create_branch("feature", &gitcomet_core::domain::CommitId("source".into()))
        .unwrap();

    let feature_target = run_git_output(repo, &["rev-parse", "feature"]);
    assert_eq!(feature_target, head);
}

#[test]
fn create_branch_succeeds_without_persisted_user_identity() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &[
            "-c",
            "user.email=you@example.com",
            "-c",
            "user.name=You",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "init",
        ],
    );

    let head = run_git_output(repo, &["rev-parse", "HEAD"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    opened
        .create_branch("feature", &gitcomet_core::domain::CommitId(head.into()))
        .unwrap();

    run_git(
        repo,
        &["show-ref", "--verify", "--quiet", "refs/heads/feature"],
    );
}

#[test]
fn checkout_branch_switches_head_to_target_branch() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    run_git(repo, &["branch", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    opened.checkout_branch("feature").unwrap();

    let head = run_git_output(repo, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(head, "feature");
}

#[test]
fn delete_branch_force_removes_unmerged_branch() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "feature.txt", "feature\n");
    run_git(repo, &["add", "feature.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );
    run_git(repo, &["checkout", "main"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let err = opened
        .delete_branch("feature")
        .expect_err("safe delete should fail for unmerged branch");
    match err.kind() {
        ErrorKind::Git(failure) => {
            assert_eq!(failure.command(), "git branch -d");
            let msg = failure.to_string();
            assert!(
                msg.contains("not fully merged") || msg.contains("cannot delete branch"),
                "unexpected delete-branch error: {msg}"
            );
        }
        other => panic!("expected structured git error, got {other:?}"),
    }

    opened.delete_branch_force("feature").unwrap();

    let deleted = git_command()
        .arg("-C")
        .arg(repo)
        .args(["show-ref", "--verify", "--quiet", "refs/heads/feature"])
        .status()
        .expect("show-ref");
    assert!(!deleted.success(), "expected force-delete to remove branch");
}

#[test]
fn delete_branch_force_removes_branch_config_section() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    run_git(repo, &["branch", "feature"]);
    run_git(repo, &["config", "branch.feature.remote", "origin"]);
    run_git(
        repo,
        &["config", "branch.feature.merge", "refs/heads/feature"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    opened.delete_branch_force("feature").unwrap();

    let branch_config = git_command()
        .arg("-C")
        .arg(repo)
        .args(["config", "--local", "--get-regexp", "^branch\\.feature\\."])
        .output()
        .expect("git config --get-regexp");
    assert_eq!(
        branch_config.status.code(),
        Some(1),
        "expected branch config section to be removed; stdout: {}; stderr: {}",
        String::from_utf8_lossy(&branch_config.stdout),
        String::from_utf8_lossy(&branch_config.stderr),
    );
}

#[test]
fn delete_branch_force_keeps_branch_config_when_local_config_is_locked() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    run_git(repo, &["branch", "feature"]);
    run_git(repo, &["config", "branch.feature.remote", "origin"]);
    run_git(
        repo,
        &["config", "branch.feature.merge", "refs/heads/feature"],
    );
    fs::write(repo.join(".git").join("config.lock"), b"held elsewhere").unwrap();

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    opened.delete_branch_force("feature").unwrap();

    let deleted = git_command()
        .arg("-C")
        .arg(repo)
        .args(["show-ref", "--verify", "--quiet", "refs/heads/feature"])
        .status()
        .expect("show-ref");
    assert!(!deleted.success(), "expected force-delete to remove branch");

    let branch_config = git_command()
        .arg("-C")
        .arg(repo)
        .args(["config", "--local", "--get-regexp", "^branch\\.feature\\."])
        .output()
        .expect("git config --get-regexp");
    assert!(
        branch_config.status.success(),
        "expected branch config section to remain when .git/config is locked"
    );
    let branch_config_stdout = String::from_utf8_lossy(&branch_config.stdout);
    assert!(
        branch_config_stdout.contains("branch.feature.remote origin")
            && branch_config_stdout.contains("branch.feature.merge refs/heads/feature"),
        "unexpected branch config after locked cleanup skip: {branch_config_stdout}"
    );
}

#[test]
fn delete_branch_force_missing_branch_is_structured_git_failure() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let err = opened
        .delete_branch_force("missing")
        .expect_err("missing branch must surface as a git command failure");
    assert_git_failure(&err, "git branch -D", GitFailureId::CommandFailed);
    let msg = err.to_string();
    assert!(
        msg.contains("branch 'missing' not found"),
        "unexpected delete-branch-force error: {msg}"
    );
}

#[test]
fn delete_branch_force_rejects_unborn_current_branch_before_missing_ref_check() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let err = opened
        .delete_branch_force("main")
        .expect_err("unborn checked-out branch must still be treated as in-use");
    assert_git_failure(&err, "git branch -D", GitFailureId::CommandFailed);
    let msg = err.to_string();
    assert!(
        msg.contains("used by worktree") && msg.contains(&git_path_arg(repo)),
        "unexpected delete-branch-force error: {msg}"
    );
}

#[test]
fn delete_branch_force_rejects_branch_checked_out_in_linked_worktree() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    let linked_worktree = dir.path().join("feature-worktree");

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    run_git(repo, &["branch", "feature"]);

    let linked_worktree_arg = git_path_arg(&linked_worktree);
    run_git(repo, &["worktree", "add", &linked_worktree_arg, "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let err = opened
        .delete_branch_force("feature")
        .expect_err("branch checked out in linked worktree must not be deleted");
    assert_git_failure(&err, "git branch -D", GitFailureId::CommandFailed);
    let msg = err.to_string();
    assert!(
        msg.contains("used by worktree") && msg.contains(&linked_worktree_arg),
        "unexpected delete-branch-force error: {msg}"
    );

    let still_exists = git_command()
        .arg("-C")
        .arg(repo)
        .args(["show-ref", "--verify", "--quiet", "refs/heads/feature"])
        .status()
        .expect("show-ref");
    assert!(
        still_exists.success(),
        "branch should remain after linked-worktree rejection"
    );
}

#[test]
fn delete_branch_force_rejects_branch_checked_out_in_main_worktree_when_opened_from_linked() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    let linked_worktree = dir.path().join("feature-worktree");

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    run_git(repo, &["branch", "feature"]);

    let linked_worktree_arg = git_path_arg(&linked_worktree);
    run_git(repo, &["worktree", "add", &linked_worktree_arg, "feature"]);

    let backend = GixBackend;
    let opened = backend.open(&linked_worktree).unwrap();
    let err = opened
        .delete_branch_force("main")
        .expect_err("main-worktree branch use must block deletion from linked worktree");
    assert_git_failure(&err, "git branch -D", GitFailureId::CommandFailed);
    let msg = err.to_string();
    assert!(
        msg.contains("used by worktree") && msg.contains(&git_path_arg(repo)),
        "unexpected delete-branch-force error: {msg}"
    );

    let still_exists = git_command()
        .arg("-C")
        .arg(repo)
        .args(["show-ref", "--verify", "--quiet", "refs/heads/main"])
        .status()
        .expect("show-ref");
    assert!(
        still_exists.success(),
        "branch should remain after main-worktree rejection"
    );
}

#[test]
fn cherry_pick_applies_commit_onto_current_branch() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "b.txt", "feature\n");
    run_git(repo, &["add", "b.txt"]);
    run_git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "feature commit",
        ],
    );
    let feature_sha = run_git_output(repo, &["rev-parse", "HEAD"]);
    run_git(repo, &["checkout", "main"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    opened
        .cherry_pick(&gitcomet_core::domain::CommitId(feature_sha.into()))
        .unwrap();

    assert_eq!(fs::read_to_string(repo.join("b.txt")).unwrap(), "feature\n");
    let status = opened.status().unwrap();
    assert!(status.staged.is_empty());
    assert!(status.unstaged.is_empty());
}

#[test]
fn interactive_cherry_pick_applies_multiple_commits_in_order() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "base.txt", "base\n");
    run_git(repo, &["add", "base.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "one.txt", "one\n");
    run_git(repo, &["add", "one.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature one"],
    );
    let one_sha = run_git_output(repo, &["rev-parse", "HEAD"]);
    write(repo, "two.txt", "two\n");
    run_git(repo, &["add", "two.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature two"],
    );
    let two_sha = run_git_output(repo, &["rev-parse", "HEAD"]);
    run_git(repo, &["checkout", "main"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    opened
        .interactive_cherry_pick_with_output(&[
            InteractiveRebaseEntry {
                action: InteractiveRebaseAction::Pick,
                commit_id: one_sha,
                summary: "feature one".to_string(),
                message: "feature one".to_string(),
                new_message: None,
            },
            InteractiveRebaseEntry {
                action: InteractiveRebaseAction::Pick,
                commit_id: two_sha,
                summary: "feature two".to_string(),
                message: "feature two".to_string(),
                new_message: None,
            },
        ])
        .unwrap();

    assert_eq!(fs::read_to_string(repo.join("one.txt")).unwrap(), "one\n");
    assert_eq!(fs::read_to_string(repo.join("two.txt")).unwrap(), "two\n");
    let subjects = run_git_output(repo, &["log", "--format=%s", "-2"]);
    assert_eq!(subjects, "feature two\nfeature one");
}

#[test]
fn create_and_delete_local_tag() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
    run_git(repo, &["config", "tag.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    // No message => lightweight tag (a ref pointing straight at the commit),
    // matching `git tag <name>` semantics.
    opened
        .create_tag_with_output("v1.0.0", "HEAD", None, false)
        .unwrap();
    run_git(
        repo,
        &["show-ref", "--verify", "--quiet", "refs/tags/v1.0.0"],
    );
    let tag_type = git_command()
        .arg("-C")
        .arg(repo)
        .args(["cat-file", "-t", "refs/tags/v1.0.0"])
        .output()
        .expect("cat-file");
    assert!(
        tag_type.status.success(),
        "expected refs/tags/v1.0.0 to exist"
    );
    assert_eq!(
        String::from_utf8_lossy(&tag_type.stdout).trim(),
        "commit",
        "a tag created without a message should be lightweight"
    );

    opened.delete_tag_with_output("v1.0.0").unwrap();
    let deleted = git_command()
        .arg("-C")
        .arg(repo)
        .args(["show-ref", "--verify", "--quiet", "refs/tags/v1.0.0"])
        .status()
        .expect("show-ref");
    assert!(!deleted.success(), "expected tag to be deleted");
}

#[test]
fn create_annotated_tag_includes_message() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
    run_git(repo, &["config", "tag.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    // A message => annotated tag object that stores the message.
    opened
        .create_tag_with_output("v1.0.0", "HEAD", Some("Release 1.0"), true)
        .unwrap();

    let tag_type = git_command()
        .arg("-C")
        .arg(repo)
        .args(["cat-file", "-t", "refs/tags/v1.0.0"])
        .output()
        .expect("cat-file");
    assert!(
        tag_type.status.success(),
        "expected refs/tags/v1.0.0 to exist"
    );
    assert_eq!(
        String::from_utf8_lossy(&tag_type.stdout).trim(),
        "tag",
        "a tag created with a message should be annotated"
    );

    let contents = git_command()
        .arg("-C")
        .arg(repo)
        .args([
            "for-each-ref",
            "--format=%(contents:subject)",
            "refs/tags/v1.0.0",
        ])
        .output()
        .expect("for-each-ref");
    assert_eq!(
        String::from_utf8_lossy(&contents.stdout).trim(),
        "Release 1.0",
        "annotated tag should carry the provided message"
    );
}

#[test]
fn create_tag_respects_tag_gpgsign_config() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
    run_git(repo, &["config", "tag.gpgsign", "true"]);
    run_git(
        repo,
        &["config", "gpg.program", "gitcomet-missing-gpg-program"],
    );

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    // Signing only applies to annotated tags, so request one with a message.
    let err = opened
        .create_tag_with_output("v1.0.0", "HEAD", Some("Release 1.0"), true)
        .expect_err("tag creation should fail when signing is required and gpg is missing");

    match err.kind() {
        ErrorKind::Git(failure) => {
            assert_eq!(failure.command(), "git tag -m <message> -- v1.0.0 HEAD");
            let msg = failure.to_string();
            assert!(
                msg.contains("git tag -m <message> -- v1.0.0 HEAD failed"),
                "unexpected git error: {msg}"
            );
            let lower = msg.to_ascii_lowercase();
            assert!(
                msg.contains("gitcomet-missing-gpg-program") || lower.contains("sign"),
                "expected signing failure details in git error: {msg}"
            );
        }
        other => panic!("expected structured git error, got {other:?}"),
    }

    let tag_present = git_command()
        .arg("-C")
        .arg(repo)
        .args(["show-ref", "--verify", "--quiet", "refs/tags/v1.0.0"])
        .status()
        .expect("show-ref");
    assert!(
        !tag_present.success(),
        "tag should not exist when signing failed"
    );
}

#[test]
fn list_tags_returns_sorted_names_with_commit_targets() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "one\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    run_git(repo, &["tag", "-a", "a-first", "-m", "a-first"]);
    run_git(repo, &["tag", "z-last"]);
    let head = run_git_output(repo, &["rev-parse", "HEAD"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let tags = opened.list_tags().unwrap();

    let names = tags.iter().map(|tag| tag.name.as_str()).collect::<Vec<_>>();
    assert_eq!(names, vec!["a-first", "z-last"]);
    assert!(tags.iter().all(|tag| tag.target.as_ref() == head));
}

#[test]
fn list_remote_tags_collects_sorted_results_and_skips_unavailable_remote() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let origin = dir.path().join("origin.git");
    let backup = dir.path().join("backup.git");
    let missing = dir.path().join("missing.git");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&origin).unwrap();
    fs::create_dir_all(&backup).unwrap();

    run_git(&repo, &["init", "-b", "main"]);
    run_git(&repo, &["config", "user.email", "you@example.com"]);
    run_git(&repo, &["config", "user.name", "You"]);
    run_git(&repo, &["config", "commit.gpgsign", "false"]);

    write(&repo, "a.txt", "one\n");
    run_git(&repo, &["add", "a.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    run_git(&origin, &["init", "--bare", "-b", "main"]);
    run_git(&backup, &["init", "--bare", "-b", "main"]);
    run_git(
        &repo,
        &["remote", "add", "origin", git_remote_url(&origin).as_str()],
    );
    run_git(
        &repo,
        &["remote", "add", "backup", git_remote_url(&backup).as_str()],
    );
    run_git(
        &repo,
        &["remote", "add", "broken", git_remote_url(&missing).as_str()],
    );

    run_git(&repo, &["tag", "origin-tag"]);
    run_git(&repo, &["tag", "backup-tag"]);
    run_git(&repo, &["push", "origin", "refs/tags/origin-tag"]);
    run_git(&repo, &["push", "backup", "refs/tags/backup-tag"]);

    let head = run_git_output(&repo, &["rev-parse", "HEAD"]);

    let backend = GixBackend;
    let opened = backend.open(&repo).unwrap();
    let remote_tags = opened.list_remote_tags().unwrap();
    let tuples = remote_tags
        .iter()
        .map(|tag| {
            (
                tag.remote.as_str(),
                tag.name.as_str(),
                tag.target.as_ref().to_string(),
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        tuples,
        vec![
            ("backup", "backup-tag", head.clone()),
            ("origin", "origin-tag", head)
        ]
    );
}

#[test]
fn push_and_delete_remote_tag() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let origin = dir.path().join("origin.git");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&origin).unwrap();

    run_git(&repo, &["init", "-b", "main"]);
    run_git(&repo, &["config", "user.email", "you@example.com"]);
    run_git(&repo, &["config", "user.name", "You"]);
    run_git(&repo, &["config", "commit.gpgsign", "false"]);
    run_git(&repo, &["config", "tag.gpgsign", "false"]);

    write(&repo, "a.txt", "one\n");
    run_git(&repo, &["add", "a.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    run_git(&origin, &["init", "--bare", "-b", "main"]);
    run_git(
        &repo,
        &["remote", "add", "origin", git_remote_url(&origin).as_str()],
    );

    let backend = GixBackend;
    let opened = backend.open(&repo).unwrap();

    opened
        .create_tag_with_output("v1.0.0", "HEAD", None, false)
        .unwrap();
    opened.push_tag_with_output("origin", "v1.0.0").unwrap();
    run_git(
        &origin,
        &["show-ref", "--verify", "--quiet", "refs/tags/v1.0.0"],
    );

    opened
        .delete_remote_tag_with_output("origin", "v1.0.0")
        .unwrap();
    let deleted = git_command()
        .arg("-C")
        .arg(&origin)
        .args(["show-ref", "--verify", "--quiet", "refs/tags/v1.0.0"])
        .status()
        .expect("show-ref");
    assert!(!deleted.success(), "expected remote tag to be deleted");
}

#[test]
fn prune_merged_branches_deletes_local_branches_missing_on_remote() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let origin = dir.path().join("origin.git");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&origin).unwrap();

    run_git(&repo, &["init", "-b", "main"]);
    run_git(&repo, &["config", "user.email", "you@example.com"]);
    run_git(&repo, &["config", "user.name", "You"]);
    run_git(&repo, &["config", "commit.gpgsign", "false"]);

    write(&repo, "a.txt", "one\n");
    run_git(&repo, &["add", "a.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    run_git(&origin, &["init", "--bare", "-b", "main"]);
    run_git(
        &repo,
        &["remote", "add", "origin", git_remote_url(&origin).as_str()],
    );
    run_git(&repo, &["push", "-u", "origin", "main"]);

    run_git(&repo, &["checkout", "-b", "feature"]);
    write(&repo, "feature.txt", "feature\n");
    run_git(&repo, &["add", "feature.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );
    run_git(&repo, &["push", "-u", "origin", "feature"]);

    run_git(&repo, &["checkout", "main"]);
    run_git(
        &repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "merge",
            "--no-ff",
            "feature",
            "-m",
            "merge feature",
        ],
    );
    run_git(&repo, &["push", "origin", "main"]);
    run_git(&repo, &["push", "origin", "--delete", "feature"]);

    run_git(
        &repo,
        &["show-ref", "--verify", "--quiet", "refs/heads/feature"],
    );

    let backend = GixBackend;
    let opened = backend.open(&repo).unwrap();
    opened.prune_merged_branches_with_output().unwrap();

    let deleted = git_command()
        .arg("-C")
        .arg(&repo)
        .args(["show-ref", "--verify", "--quiet", "refs/heads/feature"])
        .status()
        .expect("show-ref");
    assert!(
        !deleted.success(),
        "expected merged local branch to be deleted"
    );
}

#[test]
fn prune_local_tags_deletes_tags_missing_from_remotes() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let origin = dir.path().join("origin.git");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&origin).unwrap();

    run_git(&repo, &["init", "-b", "main"]);
    run_git(&repo, &["config", "user.email", "you@example.com"]);
    run_git(&repo, &["config", "user.name", "You"]);
    run_git(&repo, &["config", "commit.gpgsign", "false"]);

    write(&repo, "a.txt", "one\n");
    run_git(&repo, &["add", "a.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    run_git(&origin, &["init", "--bare", "-b", "main"]);
    run_git(
        &repo,
        &["remote", "add", "origin", git_remote_url(&origin).as_str()],
    );
    run_git(&repo, &["push", "-u", "origin", "main"]);

    run_git(&repo, &["tag", "v1.0.0"]);
    run_git(&repo, &["tag", "stale-local"]);
    run_git(&repo, &["push", "origin", "refs/tags/v1.0.0"]);
    run_git(
        &repo,
        &["show-ref", "--verify", "--quiet", "refs/tags/stale-local"],
    );

    let backend = GixBackend;
    let opened = backend.open(&repo).unwrap();
    opened.prune_local_tags_with_output().unwrap();

    run_git(
        &repo,
        &["show-ref", "--verify", "--quiet", "refs/tags/v1.0.0"],
    );
    let stale_deleted = git_command()
        .arg("-C")
        .arg(&repo)
        .args(["show-ref", "--verify", "--quiet", "refs/tags/stale-local"])
        .status()
        .expect("show-ref");
    assert!(
        !stale_deleted.success(),
        "expected stale local tag to be deleted"
    );
}

#[test]
fn prune_local_tags_with_output_no_remotes_is_noop() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).unwrap();

    run_git(&repo, &["init", "-b", "main"]);
    run_git(&repo, &["config", "user.email", "you@example.com"]);
    run_git(&repo, &["config", "user.name", "You"]);
    run_git(&repo, &["config", "commit.gpgsign", "false"]);
    write(&repo, "a.txt", "one\n");
    run_git(&repo, &["add", "a.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    run_git(&repo, &["tag", "local-only"]);

    let backend = GixBackend;
    let opened = backend.open(&repo).unwrap();
    let output = opened.prune_local_tags_with_output().unwrap();

    assert_eq!(output.exit_code, Some(0));
    assert!(
        output
            .stdout
            .contains("No remotes configured; skipping tag prune."),
        "unexpected stdout: {}",
        output.stdout
    );
    run_git(
        &repo,
        &["show-ref", "--verify", "--quiet", "refs/tags/local-only"],
    );
}

#[test]
fn prune_local_tags_with_output_reports_noop_when_all_tags_exist_remotely() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let origin = dir.path().join("origin.git");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&origin).unwrap();

    run_git(&repo, &["init", "-b", "main"]);
    run_git(&repo, &["config", "user.email", "you@example.com"]);
    run_git(&repo, &["config", "user.name", "You"]);
    run_git(&repo, &["config", "commit.gpgsign", "false"]);
    write(&repo, "a.txt", "one\n");
    run_git(&repo, &["add", "a.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    run_git(&origin, &["init", "--bare", "-b", "main"]);
    run_git(
        &repo,
        &["remote", "add", "origin", git_remote_url(&origin).as_str()],
    );
    run_git(&repo, &["push", "-u", "origin", "main"]);
    run_git(&repo, &["tag", "v1.0.0"]);
    run_git(&repo, &["push", "origin", "refs/tags/v1.0.0"]);

    let backend = GixBackend;
    let opened = backend.open(&repo).unwrap();
    let output = opened.prune_local_tags_with_output().unwrap();

    assert_eq!(output.exit_code, Some(0));
    assert!(
        output.stdout.contains("No local tags to prune."),
        "unexpected stdout: {}",
        output.stdout
    );
    run_git(
        &repo,
        &["show-ref", "--verify", "--quiet", "refs/tags/v1.0.0"],
    );
}

#[test]
fn list_remote_branches_includes_fetched_remote_tracking_refs() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let origin = dir.path().join("origin.git");
    fs::create_dir_all(&repo).unwrap();

    run_git(&repo, &["init", "-b", "main"]);
    run_git(&repo, &["config", "user.email", "you@example.com"]);
    run_git(&repo, &["config", "user.name", "You"]);
    run_git(&repo, &["config", "commit.gpgsign", "false"]);

    write(&repo, "a.txt", "one\n");
    run_git(&repo, &["add", "a.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    fs::create_dir_all(&origin).unwrap();
    run_git(&origin, &["init", "--bare", "-b", "main"]);
    run_git(
        &repo,
        &["remote", "add", "origin", git_remote_url(&origin).as_str()],
    );
    run_git(&repo, &["push", "-u", "origin", "main"]);

    run_git(&repo, &["checkout", "-b", "feature"]);
    write(&repo, "b.txt", "feature\n");
    run_git(&repo, &["add", "b.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );
    run_git(&repo, &["push", "-u", "origin", "feature"]);
    run_git(&repo, &["fetch", "origin"]);

    let backend = GixBackend;
    let opened = backend.open(&repo).unwrap();
    let branches = opened.list_remote_branches().unwrap();

    assert!(
        branches
            .iter()
            .any(|b| b.remote == "origin" && b.name == "main")
    );
    assert!(
        branches
            .iter()
            .any(|b| b.remote == "origin" && b.name == "feature")
    );
    assert!(!branches.iter().any(|b| b.name == "HEAD"));
}

#[test]
fn checkout_remote_branch_creates_tracking_branch_when_missing_locally() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let origin = dir.path().join("origin.git");
    let seed = dir.path().join("seed");
    let clone = dir.path().join("clone");
    fs::create_dir_all(&origin).unwrap();
    fs::create_dir_all(&seed).unwrap();

    run_git(&origin, &["init", "--bare", "-b", "main"]);

    run_git(&seed, &["init", "-b", "main"]);
    run_git(&seed, &["config", "user.email", "you@example.com"]);
    run_git(&seed, &["config", "user.name", "You"]);
    run_git(&seed, &["config", "commit.gpgsign", "false"]);
    write(&seed, "a.txt", "one\n");
    run_git(&seed, &["add", "a.txt"]);
    run_git(
        &seed,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    run_git(
        &seed,
        &["remote", "add", "origin", git_remote_url(&origin).as_str()],
    );
    run_git(&seed, &["push", "-u", "origin", "main"]);

    run_git(&seed, &["checkout", "-b", "feature"]);
    write(&seed, "feature.txt", "feature\n");
    run_git(&seed, &["add", "feature.txt"]);
    run_git(
        &seed,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );
    run_git(&seed, &["push", "-u", "origin", "feature"]);

    run_git(
        dir.path(),
        &[
            "clone",
            git_remote_url(&origin).as_str(),
            git_path_arg(&clone).as_str(),
        ],
    );

    let backend = GixBackend;
    let opened = backend.open(&clone).unwrap();
    opened
        .checkout_remote_branch("origin", "feature", "feature")
        .unwrap();

    let head = run_git_output(&clone, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(head, "feature");

    let upstream = run_git_output(
        &clone,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    );
    assert_eq!(upstream, "origin/feature");
}

#[test]
fn checkout_remote_branch_existing_local_branch_updates_upstream_and_checks_out() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let origin = dir.path().join("origin.git");
    let seed = dir.path().join("seed");
    let clone = dir.path().join("clone");
    fs::create_dir_all(&origin).unwrap();
    fs::create_dir_all(&seed).unwrap();

    run_git(&origin, &["init", "--bare", "-b", "main"]);

    run_git(&seed, &["init", "-b", "main"]);
    run_git(&seed, &["config", "user.email", "you@example.com"]);
    run_git(&seed, &["config", "user.name", "You"]);
    run_git(&seed, &["config", "commit.gpgsign", "false"]);
    write(&seed, "a.txt", "one\n");
    run_git(&seed, &["add", "a.txt"]);
    run_git(
        &seed,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    run_git(
        &seed,
        &["remote", "add", "origin", git_remote_url(&origin).as_str()],
    );
    run_git(&seed, &["push", "-u", "origin", "main"]);

    run_git(&seed, &["checkout", "-b", "feature"]);
    write(&seed, "feature.txt", "feature\n");
    run_git(&seed, &["add", "feature.txt"]);
    run_git(
        &seed,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );
    run_git(&seed, &["push", "-u", "origin", "feature"]);

    run_git(
        dir.path(),
        &[
            "clone",
            git_remote_url(&origin).as_str(),
            git_path_arg(&clone).as_str(),
        ],
    );
    run_git(&clone, &["checkout", "-b", "topic"]);
    run_git(&clone, &["checkout", "main"]);

    let upstream_before = git_command()
        .arg("-C")
        .arg(&clone)
        .args([
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "topic@{upstream}",
        ])
        .status()
        .expect("topic upstream probe");
    assert!(
        !upstream_before.success(),
        "topic should start without upstream tracking"
    );

    let backend = GixBackend;
    let opened = backend.open(&clone).unwrap();
    opened
        .checkout_remote_branch("origin", "feature", "topic")
        .unwrap();

    let head = run_git_output(&clone, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(head, "topic");
    let upstream = run_git_output(
        &clone,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    );
    assert_eq!(upstream, "origin/feature");
}

#[test]
fn checkout_remote_branch_sees_local_branch_created_after_backend_open() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let origin = dir.path().join("origin.git");
    let seed = dir.path().join("seed");
    let clone = dir.path().join("clone");
    fs::create_dir_all(&origin).unwrap();
    fs::create_dir_all(&seed).unwrap();

    run_git(&origin, &["init", "--bare", "-b", "main"]);

    run_git(&seed, &["init", "-b", "main"]);
    run_git(&seed, &["config", "user.email", "you@example.com"]);
    run_git(&seed, &["config", "user.name", "You"]);
    run_git(&seed, &["config", "commit.gpgsign", "false"]);
    write(&seed, "a.txt", "one\n");
    run_git(&seed, &["add", "a.txt"]);
    run_git(
        &seed,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    run_git(
        &seed,
        &["remote", "add", "origin", git_remote_url(&origin).as_str()],
    );
    run_git(&seed, &["push", "-u", "origin", "main"]);

    run_git(&seed, &["checkout", "-b", "feature"]);
    write(&seed, "feature.txt", "feature\n");
    run_git(&seed, &["add", "feature.txt"]);
    run_git(
        &seed,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );
    run_git(&seed, &["push", "-u", "origin", "feature"]);

    run_git(
        dir.path(),
        &[
            "clone",
            git_remote_url(&origin).as_str(),
            git_path_arg(&clone).as_str(),
        ],
    );

    let backend = GixBackend;
    let opened = backend.open(&clone).unwrap();

    run_git(&clone, &["checkout", "-b", "topic"]);
    run_git(&clone, &["checkout", "main"]);

    let upstream_before = git_command()
        .arg("-C")
        .arg(&clone)
        .args([
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "topic@{upstream}",
        ])
        .status()
        .expect("topic upstream probe");
    assert!(
        !upstream_before.success(),
        "topic should start without upstream tracking"
    );

    opened
        .checkout_remote_branch("origin", "feature", "topic")
        .unwrap();

    let head = run_git_output(&clone, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(head, "topic");
    let upstream = run_git_output(
        &clone,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    );
    assert_eq!(upstream, "origin/feature");
}

#[test]
fn checkout_remote_branch_returns_structured_git_error_for_missing_remote_branch() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let origin = dir.path().join("origin.git");
    let repo = dir.path().join("repo");
    fs::create_dir_all(&origin).unwrap();
    fs::create_dir_all(&repo).unwrap();

    run_git(&origin, &["init", "--bare", "-b", "main"]);

    run_git(&repo, &["init", "-b", "main"]);
    run_git(&repo, &["config", "user.email", "you@example.com"]);
    run_git(&repo, &["config", "user.name", "You"]);
    run_git(&repo, &["config", "commit.gpgsign", "false"]);
    write(&repo, "a.txt", "one\n");
    run_git(&repo, &["add", "a.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    run_git(
        &repo,
        &["remote", "add", "origin", git_remote_url(&origin).as_str()],
    );
    run_git(&repo, &["push", "-u", "origin", "main"]);
    run_git(&repo, &["fetch", "origin"]);

    let backend = GixBackend;
    let opened = backend.open(&repo).unwrap();
    let err = opened
        .checkout_remote_branch("origin", "missing-branch", "topic")
        .expect_err("missing remote branch should return structured git error");
    match err.kind() {
        ErrorKind::Git(failure) => {
            assert_eq!(failure.id(), GitFailureId::CommandFailed);
            assert_eq!(failure.command(), "git checkout --track");
            assert!(
                failure.exit_code().is_some(),
                "git checkout failure should preserve exit code"
            );
            assert!(
                failure
                    .detail()
                    .is_some_and(|detail| !detail.trim().is_empty()),
                "git checkout failure should preserve stderr detail"
            );
        }
        other => panic!("expected structured git error, got {other:?}"),
    }
}

#[test]
fn checkout_remote_branch_with_existing_local_branch_and_missing_remote_keeps_head_unchanged() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let origin = dir.path().join("origin.git");
    let seed = dir.path().join("seed");
    let clone = dir.path().join("clone");
    fs::create_dir_all(&origin).unwrap();
    fs::create_dir_all(&seed).unwrap();

    run_git(&origin, &["init", "--bare", "-b", "main"]);

    run_git(&seed, &["init", "-b", "main"]);
    run_git(&seed, &["config", "user.email", "you@example.com"]);
    run_git(&seed, &["config", "user.name", "You"]);
    run_git(&seed, &["config", "commit.gpgsign", "false"]);
    write(&seed, "a.txt", "one\n");
    run_git(&seed, &["add", "a.txt"]);
    run_git(
        &seed,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    run_git(
        &seed,
        &["remote", "add", "origin", git_remote_url(&origin).as_str()],
    );
    run_git(&seed, &["push", "-u", "origin", "main"]);

    run_git(
        dir.path(),
        &[
            "clone",
            git_remote_url(&origin).as_str(),
            git_path_arg(&clone).as_str(),
        ],
    );
    run_git(&clone, &["checkout", "-b", "topic"]);
    run_git(&clone, &["checkout", "main"]);

    let backend = GixBackend;
    let opened = backend.open(&clone).unwrap();
    let err = opened
        .checkout_remote_branch("origin", "missing-branch", "topic")
        .expect_err("missing remote branch should not switch to the existing local branch");
    assert_git_failure(&err, "git checkout --track", GitFailureId::CommandFailed);

    let head = run_git_output(&clone, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(head, "main");

    let upstream = git_command()
        .arg("-C")
        .arg(&clone)
        .args([
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "topic@{upstream}",
        ])
        .status()
        .expect("topic upstream probe");
    assert!(
        !upstream.success(),
        "topic should remain without upstream tracking after the failed checkout"
    );
}

#[test]
fn checkout_remote_branch_dirty_worktree_failure_does_not_create_local_branch() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let origin = dir.path().join("origin.git");
    let seed = dir.path().join("seed");
    let clone = dir.path().join("clone");
    fs::create_dir_all(&origin).unwrap();
    fs::create_dir_all(&seed).unwrap();

    run_git(&origin, &["init", "--bare", "-b", "main"]);

    run_git(&seed, &["init", "-b", "main"]);
    run_git(&seed, &["config", "user.email", "you@example.com"]);
    run_git(&seed, &["config", "user.name", "You"]);
    run_git(&seed, &["config", "commit.gpgsign", "false"]);
    write(&seed, "a.txt", "one\n");
    run_git(&seed, &["add", "a.txt"]);
    run_git(
        &seed,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    run_git(
        &seed,
        &["remote", "add", "origin", git_remote_url(&origin).as_str()],
    );
    run_git(&seed, &["push", "-u", "origin", "main"]);

    run_git(&seed, &["checkout", "-b", "feature"]);
    write(&seed, "a.txt", "feature\n");
    run_git(&seed, &["add", "a.txt"]);
    run_git(
        &seed,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );
    run_git(&seed, &["push", "-u", "origin", "feature"]);

    run_git(
        dir.path(),
        &[
            "clone",
            git_remote_url(&origin).as_str(),
            git_path_arg(&clone).as_str(),
        ],
    );
    write(&clone, "a.txt", "dirty\n");

    let backend = GixBackend;
    let opened = backend.open(&clone).unwrap();
    let err = opened
        .checkout_remote_branch("origin", "feature", "topic")
        .expect_err("dirty checkout should fail");
    assert_git_failure(&err, "git checkout --track", GitFailureId::CommandFailed);

    let topic_exists = git_command()
        .arg("-C")
        .arg(&clone)
        .args(["show-ref", "--verify", "--quiet", "refs/heads/topic"])
        .status()
        .expect("show-ref topic");
    assert!(
        !topic_exists.success(),
        "topic branch should not be created when checkout fails"
    );

    let head = run_git_output(&clone, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(head, "main");
    assert_eq!(fs::read_to_string(clone.join("a.txt")).unwrap(), "dirty\n");
}

#[test]
fn push_with_output_updates_remote_head() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let origin = dir.path().join("origin.git");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&origin).unwrap();

    run_git(&repo, &["init", "-b", "main"]);
    run_git(&repo, &["config", "user.email", "you@example.com"]);
    run_git(&repo, &["config", "user.name", "You"]);
    run_git(&repo, &["config", "commit.gpgsign", "false"]);

    write(&repo, "a.txt", "one\n");
    run_git(&repo, &["add", "a.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    run_git(&origin, &["init", "--bare", "-b", "main"]);
    run_git(
        &repo,
        &["remote", "add", "origin", git_remote_url(&origin).as_str()],
    );
    run_git(&repo, &["push", "-u", "origin", "main"]);

    write(&repo, "a.txt", "one\ntwo\n");
    run_git(&repo, &["add", "a.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "second"],
    );
    let head_local = git_command()
        .arg("-C")
        .arg(&repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse HEAD");
    assert!(head_local.status.success());
    let head_local = String::from_utf8(head_local.stdout)
        .unwrap()
        .trim()
        .to_string();

    let backend = GixBackend;
    let opened = backend.open(&repo).unwrap();
    opened.push_with_output().unwrap();

    let head_remote = git_command()
        .arg("-C")
        .arg(&origin)
        .args(["rev-parse", "refs/heads/main"])
        .output()
        .expect("rev-parse origin/main");
    assert!(head_remote.status.success());
    let head_remote = String::from_utf8(head_remote.stdout)
        .unwrap()
        .trim()
        .to_string();
    assert_eq!(head_remote, head_local);
}

#[test]
fn force_push_with_output_updates_remote_head_after_rewrite() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let origin = dir.path().join("origin.git");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&origin).unwrap();

    run_git(&repo, &["init", "-b", "main"]);
    run_git(&repo, &["config", "user.email", "you@example.com"]);
    run_git(&repo, &["config", "user.name", "You"]);
    run_git(&repo, &["config", "commit.gpgsign", "false"]);

    write(&repo, "a.txt", "one\n");
    run_git(&repo, &["add", "a.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    run_git(&origin, &["init", "--bare", "-b", "main"]);
    run_git(
        &repo,
        &["remote", "add", "origin", git_remote_url(&origin).as_str()],
    );
    run_git(&repo, &["push", "-u", "origin", "main"]);

    write(&repo, "a.txt", "one\ntwo\n");
    run_git(&repo, &["add", "a.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "second"],
    );
    run_git(&repo, &["push"]);
    run_git(&repo, &["fetch", "origin"]);

    // Rewrite local history so it diverges from the remote.
    run_git(&repo, &["reset", "--hard", "HEAD~1"]);
    write(&repo, "a.txt", "one\ntwo (rewritten)\n");
    run_git(&repo, &["add", "a.txt"]);
    run_git(
        &repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "second rewritten",
        ],
    );
    let head_local = git_command()
        .arg("-C")
        .arg(&repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse HEAD");
    assert!(head_local.status.success());
    let head_local = String::from_utf8(head_local.stdout)
        .unwrap()
        .trim()
        .to_string();

    let backend = GixBackend;
    let opened = backend.open(&repo).unwrap();
    opened.push_force_with_output().unwrap();

    let head_remote = git_command()
        .arg("-C")
        .arg(&origin)
        .args(["rev-parse", "refs/heads/main"])
        .output()
        .expect("rev-parse refs/heads/main");
    assert!(head_remote.status.success());
    let head_remote = String::from_utf8(head_remote.stdout)
        .unwrap()
        .trim()
        .to_string();
    assert_eq!(head_remote, head_local);
}

#[test]
fn pull_with_output_fast_forwards_from_remote() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let origin = dir.path().join("origin.git");
    let repo_a = dir.path().join("repo-a");
    let repo_b = dir.path().join("repo-b");
    fs::create_dir_all(&origin).unwrap();
    fs::create_dir_all(&repo_a).unwrap();

    run_git(&origin, &["init", "--bare", "-b", "main"]);

    run_git(&repo_a, &["init", "-b", "main"]);
    run_git(&repo_a, &["config", "user.email", "you@example.com"]);
    run_git(&repo_a, &["config", "user.name", "You"]);
    run_git(&repo_a, &["config", "commit.gpgsign", "false"]);
    write(&repo_a, "a.txt", "one\n");
    run_git(&repo_a, &["add", "a.txt"]);
    run_git(
        &repo_a,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    run_git(
        &repo_a,
        &["remote", "add", "origin", git_remote_url(&origin).as_str()],
    );
    run_git(&repo_a, &["push", "-u", "origin", "main"]);

    run_git(
        dir.path(),
        &[
            "clone",
            git_remote_url(&origin).as_str(),
            git_path_arg(&repo_b).as_str(),
        ],
    );

    write(&repo_a, "a.txt", "one\ntwo\n");
    run_git(&repo_a, &["add", "a.txt"]);
    run_git(
        &repo_a,
        &["-c", "commit.gpgsign=false", "commit", "-m", "second"],
    );
    run_git(&repo_a, &["push"]);

    let head_origin = git_command()
        .arg("-C")
        .arg(&origin)
        .args(["rev-parse", "refs/heads/main"])
        .output()
        .expect("rev-parse origin");
    assert!(head_origin.status.success());
    let head_origin = String::from_utf8(head_origin.stdout)
        .unwrap()
        .trim()
        .to_string();

    let backend = GixBackend;
    let opened_b = backend.open(&repo_b).unwrap();
    opened_b
        .pull_with_output(gitcomet_core::services::PullMode::FastForwardOnly)
        .unwrap();

    let head_b = git_command()
        .arg("-C")
        .arg(&repo_b)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse b");
    assert!(head_b.status.success());
    let head_b = String::from_utf8(head_b.stdout).unwrap().trim().to_string();
    assert_eq!(head_b, head_origin);
}

#[test]
fn pull_with_output_fast_forwards_when_possible_even_if_pull_ff_is_disabled() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let origin = dir.path().join("origin.git");
    let repo_a = dir.path().join("repo-a");
    let repo_b = dir.path().join("repo-b");
    fs::create_dir_all(&origin).unwrap();
    fs::create_dir_all(&repo_a).unwrap();

    run_git(&origin, &["init", "--bare", "-b", "main"]);

    run_git(&repo_a, &["init", "-b", "main"]);
    run_git(&repo_a, &["config", "user.email", "you@example.com"]);
    run_git(&repo_a, &["config", "user.name", "You"]);
    run_git(&repo_a, &["config", "commit.gpgsign", "false"]);
    write(&repo_a, "a.txt", "one\n");
    run_git(&repo_a, &["add", "a.txt"]);
    run_git(
        &repo_a,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    run_git(
        &repo_a,
        &["remote", "add", "origin", git_remote_url(&origin).as_str()],
    );
    run_git(&repo_a, &["push", "-u", "origin", "main"]);

    run_git(
        dir.path(),
        &[
            "clone",
            git_remote_url(&origin).as_str(),
            git_path_arg(&repo_b).as_str(),
        ],
    );

    run_git(&repo_b, &["config", "user.email", "you@example.com"]);
    run_git(&repo_b, &["config", "user.name", "You"]);
    run_git(&repo_b, &["config", "commit.gpgsign", "false"]);
    run_git(&repo_b, &["config", "pull.ff", "false"]);

    write(&repo_a, "a.txt", "one\ntwo\n");
    run_git(&repo_a, &["add", "a.txt"]);
    run_git(
        &repo_a,
        &["-c", "commit.gpgsign=false", "commit", "-m", "second"],
    );
    run_git(&repo_a, &["push"]);

    let head_origin = git_command()
        .arg("-C")
        .arg(&origin)
        .args(["rev-parse", "refs/heads/main"])
        .output()
        .expect("rev-parse origin");
    assert!(head_origin.status.success());
    let head_origin = String::from_utf8(head_origin.stdout)
        .unwrap()
        .trim()
        .to_string();

    let backend = GixBackend;
    let opened_b = backend.open(&repo_b).unwrap();
    opened_b
        .pull_with_output(gitcomet_core::services::PullMode::Merge)
        .unwrap();

    let head_b = git_command()
        .arg("-C")
        .arg(&repo_b)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse b");
    assert!(head_b.status.success());
    let head_b = String::from_utf8(head_b.stdout).unwrap().trim().to_string();
    assert_eq!(head_b, head_origin);

    let parents = git_command()
        .arg("-C")
        .arg(&repo_b)
        .args(["rev-list", "--parents", "-n", "1", "HEAD"])
        .output()
        .expect("rev-list --parents");
    assert!(parents.status.success());
    let parent_count = String::from_utf8(parents.stdout)
        .unwrap()
        .split_whitespace()
        .count()
        .saturating_sub(1);
    assert_eq!(parent_count, 1, "expected fast-forward");
}
