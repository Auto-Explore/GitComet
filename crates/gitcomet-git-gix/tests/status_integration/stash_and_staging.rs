use super::*;

#[test]
fn stash_create_list_apply_and_drop_work() {
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

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened.stash_create("wip", false).unwrap();
    assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "one\n");

    let stashes = opened.stash_list().unwrap();
    assert!(!stashes.is_empty());
    assert_eq!(stashes[0].index, 0);
    assert!(stashes[0].message.contains("wip"));

    opened.stash_apply(0).unwrap();
    assert_eq!(
        fs::read_to_string(repo.join("a.txt")).unwrap(),
        "one\ntwo\n"
    );

    opened.stash_drop(0).unwrap();
    let stashes = opened.stash_list().unwrap();
    assert!(stashes.is_empty());
}

#[test]
fn stash_apply_conflict_is_mergeable() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\nline\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    write(repo, "a.txt", "base\nstash-change\n");
    opened.stash_create("wip", false).unwrap();

    write(repo, "a.txt", "base\nbranch-change\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "branch-change",
        ],
    );

    let err = opened
        .stash_apply(0)
        .expect_err("stash apply conflict should report failure");
    assert_git_failure(&err, "git stash apply", GitFailureId::StashApplyConflict);
    assert!(
        err.to_string().contains("git stash apply failed"),
        "unexpected error: {err}"
    );

    let status = opened.status().unwrap();
    let conflict_entry = status
        .unstaged
        .iter()
        .find(|entry| entry.path == Path::new("a.txt"))
        .expect("expected conflicted path after stash apply merge");
    assert_eq!(conflict_entry.kind, FileStatusKind::Conflicted);
    assert_eq!(
        conflict_entry.conflict,
        Some(FileConflictKind::BothModified)
    );

    let contents = fs::read_to_string(repo.join("a.txt")).unwrap();
    assert!(contents.contains("<<<<<<<"));
    assert!(contents.contains("======="));
    assert!(contents.contains(">>>>>>>"));
}

#[test]
fn stash_apply_still_errors_when_merge_does_not_start() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\nline\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    write(repo, "a.txt", "base\nstash-change\n");
    opened.stash_create("wip", false).unwrap();

    write(repo, "a.txt", "base\nlocal-uncommitted-change\n");

    let err = opened
        .stash_apply(0)
        .expect_err("stash apply should fail when local edits would be overwritten");
    assert_git_failure(
        &err,
        "git stash apply",
        GitFailureId::WorktreeWouldBeOverwritten,
    );
    assert!(
        err.to_string().contains("overwritten by merge"),
        "unexpected error: {err}"
    );

    let status = opened.status().unwrap();
    let entry = status
        .unstaged
        .iter()
        .find(|candidate| candidate.path == Path::new("a.txt"))
        .expect("expected modified file in unstaged status");
    assert_eq!(entry.kind, FileStatusKind::Modified);
    assert_eq!(entry.conflict, None);
}

#[test]
fn stash_apply_tracked_payload_overwriting_untracked_file_is_worktree_overwrite() {
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
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    write(repo, "new.txt", "from stash\n");
    run_git(repo, &["add", "new.txt"]);
    opened.stash_create("wip", false).unwrap();

    write(repo, "new.txt", "local untracked\n");

    let err = opened.stash_apply(0).expect_err(
        "stash apply should fail when tracked stash payload would overwrite an untracked file",
    );
    assert_git_failure(
        &err,
        "git stash apply",
        GitFailureId::WorktreeWouldBeOverwritten,
    );
    assert!(
        err.to_string().contains("overwritten by merge"),
        "unexpected error: {err}"
    );

    assert_eq!(
        fs::read_to_string(repo.join("new.txt")).unwrap(),
        "local untracked\n"
    );
    let status = opened.status().unwrap();
    let entry = status
        .unstaged
        .iter()
        .find(|candidate| candidate.path == Path::new("new.txt"))
        .expect("expected blocked untracked file to remain in the worktree");
    assert_eq!(entry.kind, FileStatusKind::Untracked);
    assert_eq!(entry.conflict, None);
}

#[test]
fn stash_apply_staged_overlap_still_merges_into_conflict() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\nline\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    write(repo, "a.txt", "base\nstash-change\n");
    opened.stash_create("wip", false).unwrap();

    write(repo, "a.txt", "base\nlocal-staged-change\n");
    run_git(repo, &["add", "a.txt"]);

    let err = opened
        .stash_apply(0)
        .expect_err("stash apply should report a conflict when only the index overlaps");
    assert_git_failure(&err, "git stash apply", GitFailureId::StashApplyConflict);

    let status = opened.status().unwrap();
    let entry = status
        .unstaged
        .iter()
        .find(|candidate| candidate.path == Path::new("a.txt"))
        .expect("expected conflicted file after stash apply merge");
    assert_eq!(entry.kind, FileStatusKind::Conflicted);
    assert_eq!(entry.conflict, Some(FileConflictKind::BothModified));
}

#[test]
fn stash_apply_allows_merge_when_only_untracked_restore_fails() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\nline\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    write(repo, "a.txt", "base\nstash-change\n");
    write(repo, "Cargo.toml.orig", "from stash\n");
    opened.stash_create("wip", true).unwrap();

    // Existing untracked file blocks restoration of untracked payload from stash.
    write(repo, "Cargo.toml.orig", "local copy\n");

    let err = opened
        .stash_apply(0)
        .expect_err("stash apply should report untracked restore failure");
    assert_git_failure(
        &err,
        "git stash apply",
        GitFailureId::UntrackedRestoreConflict,
    );
    assert!(
        err.to_string()
            .contains("could not restore untracked files from stash")
            || err.to_string().contains("already exists, no checkout"),
        "unexpected error: {err}"
    );

    assert_eq!(
        fs::read_to_string(repo.join("a.txt")).unwrap(),
        "base\nstash-change\n"
    );
    let untracked_merged = fs::read_to_string(repo.join("Cargo.toml.orig")).unwrap();
    assert!(untracked_merged.contains("<<<<<<< Current file"));
    assert!(untracked_merged.contains("local copy"));
    assert!(untracked_merged.contains("======="));
    assert!(untracked_merged.contains("from stash"));
    assert!(untracked_merged.contains(">>>>>>> Stashed file"));

    let status = opened.status().unwrap();
    let tracked = status
        .unstaged
        .iter()
        .find(|candidate| candidate.path == Path::new("a.txt"))
        .expect("expected tracked stash change to be present");
    assert_eq!(tracked.kind, FileStatusKind::Modified);
    assert_eq!(tracked.conflict, None);
    assert!(status.unstaged.iter().any(|candidate| {
        candidate.path == Path::new("Cargo.toml.orig")
            && candidate.kind == FileStatusKind::Untracked
    }));
}

#[test]
fn stash_apply_preserves_original_error_when_untracked_merge_markers_fail() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\nline\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    write(repo, "Cargo.toml.orig", "from stash\n");
    opened.stash_create("wip", true).unwrap();

    let local_binary = b"\xff\xfe\x00\x80";
    write(repo, "Cargo.toml.orig", local_binary);

    let err = opened
        .stash_apply(0)
        .expect_err("stash apply should still report the original untracked restore failure");
    assert_git_failure(
        &err,
        "git stash apply",
        GitFailureId::UntrackedRestoreConflict,
    );
    assert!(
        !err.to_string().contains("cannot merge binary"),
        "unexpected recovery error replaced the original stash failure: {err}"
    );
    assert_eq!(
        fs::read(repo.join("Cargo.toml.orig")).unwrap(),
        local_binary,
    );
}

#[test]
fn stash_apply_allows_untracked_restore_failure_when_stash_has_tracked_payload() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\nline\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    // Stash contains tracked and untracked payload.
    write(repo, "a.txt", "base\nstash-change\n");
    write(repo, "Cargo.toml.orig", "from stash\n");
    opened.stash_create("wip", true).unwrap();

    // Apply the same tracked change on the branch first, so stash apply has no
    // tracked-status delta even though stash had tracked payload.
    write(repo, "a.txt", "base\nstash-change\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "same-tracked-change",
        ],
    );

    // Existing untracked file blocks restoration of stash untracked payload.
    write(repo, "Cargo.toml.orig", "local copy\n");

    let err = opened
        .stash_apply(0)
        .expect_err("stash apply should report untracked restore failure");
    assert_git_failure(
        &err,
        "git stash apply",
        GitFailureId::UntrackedRestoreConflict,
    );
    assert!(
        err.to_string()
            .contains("could not restore untracked files from stash")
            || err.to_string().contains("already exists, no checkout"),
        "unexpected error: {err}"
    );

    assert_eq!(
        fs::read_to_string(repo.join("a.txt")).unwrap(),
        "base\nstash-change\n"
    );
    let untracked_merged = fs::read_to_string(repo.join("Cargo.toml.orig")).unwrap();
    assert!(untracked_merged.contains("<<<<<<< Current file"));
    assert!(untracked_merged.contains("local copy"));
    assert!(untracked_merged.contains("======="));
    assert!(untracked_merged.contains("from stash"));
    assert!(untracked_merged.contains(">>>>>>> Stashed file"));

    let status = opened.status().unwrap();
    assert!(
        status
            .unstaged
            .iter()
            .all(|entry| entry.path != Path::new("a.txt"))
    );
    assert!(status.unstaged.iter().any(|candidate| {
        candidate.path == Path::new("Cargo.toml.orig")
            && candidate.kind == FileStatusKind::Untracked
    }));
}

#[test]
fn stash_apply_merges_when_only_untracked_restore_fails_without_tracked_changes() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base\nline\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    write(repo, "Cargo.toml.orig", "from stash\n");
    opened.stash_create("wip", true).unwrap();

    write(repo, "Cargo.toml.orig", "local copy\n");

    let err = opened
        .stash_apply(0)
        .expect_err("stash apply should report untracked restore failure");
    assert_git_failure(
        &err,
        "git stash apply",
        GitFailureId::UntrackedRestoreConflict,
    );
    assert!(
        err.to_string()
            .contains("could not restore untracked files from stash")
            || err.to_string().contains("already exists, no checkout"),
        "unexpected error: {err}"
    );

    let contents = fs::read_to_string(repo.join("Cargo.toml.orig")).unwrap();
    assert!(contents.contains("<<<<<<< Current file"));
    assert!(contents.contains("local copy"));
    assert!(contents.contains("======="));
    assert!(contents.contains("from stash"));
    assert!(contents.contains(">>>>>>> Stashed file"));

    let status = opened.status().unwrap();
    assert!(status.unstaged.iter().any(|entry| {
        entry.path == Path::new("Cargo.toml.orig") && entry.kind == FileStatusKind::Untracked
    }));
}

#[test]
fn stash_list_reports_reflog_indices_for_drop() {
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
    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    opened.stash_create("wip-1", false).unwrap();

    write(repo, "a.txt", "one\nthree\n");
    opened.stash_create("wip-2", false).unwrap();

    let stashes = opened.stash_list().unwrap();
    assert_eq!(stashes.len(), 2);
    assert_eq!(stashes[0].index, 0);
    assert_eq!(stashes[1].index, 1);

    // Drop the older stash by the index returned from `stash_list`.
    opened.stash_drop(stashes[1].index).unwrap();
    let stashes = opened.stash_list().unwrap();
    assert_eq!(stashes.len(), 1);
    assert_eq!(stashes[0].index, 0);
    assert!(stashes[0].message.contains("wip-2"));
}

#[test]
fn checkout_commit_detaches_head_at_target() {
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

    let sha = git_command()
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse HEAD");
    assert!(sha.status.success());
    let sha = String::from_utf8(sha.stdout).unwrap().trim().to_string();

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    opened
        .checkout_commit(&gitcomet_core::domain::CommitId(sha.clone().into()))
        .unwrap();

    let head_name = git_command()
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .expect("rev-parse --abbrev-ref");
    assert!(head_name.status.success());
    assert_eq!(String::from_utf8(head_name.stdout).unwrap().trim(), "HEAD");

    let head_sha = git_command()
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("rev-parse head sha");
    assert!(head_sha.status.success());
    assert_eq!(String::from_utf8(head_sha.stdout).unwrap().trim(), sha);
}

#[test]
fn discard_worktree_changes_reverts_to_index_version() {
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
    write(repo, "a.txt", "one\ntwo\nthree\n");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened
        .discard_worktree_changes(&[Path::new("a.txt")])
        .unwrap();

    assert_eq!(
        fs::read_to_string(repo.join("a.txt")).unwrap(),
        "one\ntwo\n"
    );

    let status = opened.status().unwrap();
    assert!(
        status
            .staged
            .iter()
            .any(|e| e.path == Path::new("a.txt") && e.kind == FileStatusKind::Modified)
    );
    assert!(!status.unstaged.iter().any(|e| e.path == Path::new("a.txt")));
}

#[test]
fn discard_worktree_changes_reverts_modified_file_to_head() {
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

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened
        .discard_worktree_changes(&[Path::new("a.txt")])
        .unwrap();

    assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "one\n");
    let status = opened.status().unwrap();
    assert!(status.staged.is_empty());
    assert!(status.unstaged.is_empty());
}

#[test]
fn discard_worktree_changes_removes_staged_new_file() {
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

    write(repo, "new.txt", "new\n");
    run_git(repo, &["add", "new.txt"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened
        .discard_worktree_changes(&[Path::new("new.txt")])
        .unwrap();

    assert!(!repo.join("new.txt").exists());
    let status = opened.status().unwrap();
    assert!(!status.staged.iter().any(|e| e.path == Path::new("new.txt")));
    assert!(
        !status
            .unstaged
            .iter()
            .any(|e| e.path == Path::new("new.txt"))
    );
}

#[test]
fn discard_worktree_changes_removes_untracked_file() {
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

    write(repo, "untracked.txt", "new\n");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened
        .discard_worktree_changes(&[Path::new("untracked.txt")])
        .unwrap();

    assert!(!repo.join("untracked.txt").exists());
    let status = opened.status().unwrap();
    assert!(
        !status
            .unstaged
            .iter()
            .any(|e| e.path == Path::new("untracked.txt"))
    );
}

#[test]
fn discard_worktree_changes_supports_mixed_selection() {
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
    write(repo, "b.txt", "two\n");
    run_git(repo, &["add", "a.txt", "b.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    write(repo, "a.txt", "one!\n");
    fs::remove_file(repo.join("b.txt")).unwrap();
    write(repo, "c.txt", "three\n");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    opened
        .discard_worktree_changes(&[Path::new("a.txt"), Path::new("b.txt"), Path::new("c.txt")])
        .unwrap();

    assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "one\n");
    assert_eq!(fs::read_to_string(repo.join("b.txt")).unwrap(), "two\n");
    assert!(!repo.join("c.txt").exists());
    let status = opened.status().unwrap();
    assert!(status.staged.is_empty());
    assert!(status.unstaged.is_empty());
}

#[test]
fn stage_hunk_applies_only_part_of_a_file_to_index() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    let mut base = String::new();
    for i in 1..=30 {
        base.push_str(&format!("L{i:02}\n"));
    }
    write(repo, "a.txt", &base);
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    let modified = base
        .replace("L02\n", "L02-mod\n")
        .replace("L25\n", "L25-mod\n");
    write(repo, "a.txt", &modified);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let unstaged_before = opened
        .diff_unified(&DiffTarget::WorkingTree {
            path: PathBuf::from("a.txt"),
            area: DiffArea::Unstaged,
        })
        .unwrap();
    let hunk_count_before = unstaged_before
        .lines()
        .filter(|l| l.starts_with("@@"))
        .count();
    assert_eq!(
        hunk_count_before, 2,
        "expected two hunks:\n{unstaged_before}"
    );

    let lines = unstaged_before.lines().collect::<Vec<_>>();
    let file_start = lines
        .iter()
        .position(|l| l.starts_with("diff --git "))
        .unwrap_or(0);
    let first_hunk = lines
        .iter()
        .position(|l| l.starts_with("@@"))
        .expect("first hunk header");
    let second_hunk = (first_hunk + 1..lines.len())
        .find(|&ix| lines.get(ix).is_some_and(|l| l.starts_with("@@")))
        .expect("second hunk header");

    let patch = lines[file_start..first_hunk]
        .iter()
        .chain(lines[first_hunk..second_hunk].iter())
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    opened
        .apply_unified_patch_to_index_with_output(&patch, false)
        .unwrap();

    let staged_after = opened
        .diff_unified(&DiffTarget::WorkingTree {
            path: PathBuf::from("a.txt"),
            area: DiffArea::Staged,
        })
        .unwrap();
    assert_eq!(
        staged_after.lines().filter(|l| l.starts_with("@@")).count(),
        1,
        "expected one staged hunk:\n{staged_after}"
    );
    assert!(staged_after.contains("-L02"));
    assert!(staged_after.contains("+L02-mod"));
    assert!(!staged_after.contains("L25-mod"));

    let unstaged_after = opened
        .diff_unified(&DiffTarget::WorkingTree {
            path: PathBuf::from("a.txt"),
            area: DiffArea::Unstaged,
        })
        .unwrap();
    assert_eq!(
        unstaged_after
            .lines()
            .filter(|l| l.starts_with("@@"))
            .count(),
        1,
        "expected one remaining unstaged hunk:\n{unstaged_after}"
    );
    assert!(!unstaged_after.contains("L02-mod"));
    assert!(unstaged_after.contains("-L25"));
    assert!(unstaged_after.contains("+L25-mod"));
}

#[test]
fn unstage_hunk_reverts_only_that_part_in_index() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    let mut base = String::new();
    for i in 1..=30 {
        base.push_str(&format!("L{i:02}\n"));
    }
    write(repo, "a.txt", &base);
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    let modified = base
        .replace("L02\n", "L02-mod\n")
        .replace("L25\n", "L25-mod\n");
    write(repo, "a.txt", &modified);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let unstaged_before = opened
        .diff_unified(&DiffTarget::WorkingTree {
            path: PathBuf::from("a.txt"),
            area: DiffArea::Unstaged,
        })
        .unwrap();
    assert_eq!(
        unstaged_before
            .lines()
            .filter(|l| l.starts_with("@@"))
            .count(),
        2,
        "expected two hunks:\n{unstaged_before}"
    );

    let lines = unstaged_before.lines().collect::<Vec<_>>();
    let file_start = lines
        .iter()
        .position(|l| l.starts_with("diff --git "))
        .unwrap_or(0);
    let first_hunk = lines
        .iter()
        .position(|l| l.starts_with("@@"))
        .expect("first hunk header");
    let second_hunk = (first_hunk + 1..lines.len())
        .find(|&ix| lines.get(ix).is_some_and(|l| l.starts_with("@@")))
        .expect("second hunk header");

    let patch = lines[file_start..first_hunk]
        .iter()
        .chain(lines[first_hunk..second_hunk].iter())
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";

    opened
        .apply_unified_patch_to_index_with_output(&patch, false)
        .unwrap();

    let staged_after_stage = opened
        .diff_unified(&DiffTarget::WorkingTree {
            path: PathBuf::from("a.txt"),
            area: DiffArea::Staged,
        })
        .unwrap();
    assert_eq!(
        staged_after_stage
            .lines()
            .filter(|l| l.starts_with("@@"))
            .count(),
        1,
        "expected one staged hunk:\n{staged_after_stage}"
    );

    opened
        .apply_unified_patch_to_index_with_output(&patch, true)
        .unwrap();

    let staged_after_unstage = opened
        .diff_unified(&DiffTarget::WorkingTree {
            path: PathBuf::from("a.txt"),
            area: DiffArea::Staged,
        })
        .unwrap();
    assert!(
        staged_after_unstage.trim().is_empty(),
        "expected staged diff to be empty:\n{staged_after_unstage}"
    );

    let unstaged_after_unstage = opened
        .diff_unified(&DiffTarget::WorkingTree {
            path: PathBuf::from("a.txt"),
            area: DiffArea::Unstaged,
        })
        .unwrap();
    assert_eq!(
        unstaged_after_unstage
            .lines()
            .filter(|l| l.starts_with("@@"))
            .count(),
        2,
        "expected two unstaged hunks:\n{unstaged_after_unstage}"
    );
    assert!(unstaged_after_unstage.contains("+L02-mod"));
    assert!(unstaged_after_unstage.contains("+L25-mod"));
}

/// Unstaging must not disturb a merge in progress: a bare `git reset` collapses
/// unmerged index entries and clears MERGE_HEAD, which turns conflicted files
/// into ordinary modifications still full of conflict markers.
#[test]
fn unstage_all_leaves_conflicted_paths_and_the_merge_alone() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "c.txt", "base\n");
    write(repo, "other.txt", "other\n");
    run_git(repo, &["add", "."]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    let base_branch = run_git_output(repo, &["rev-parse", "--abbrev-ref", "HEAD"]);
    let base_branch = base_branch.trim().to_string();

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "c.txt", "theirs\n");
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-am", "theirs"],
    );
    run_git(repo, &["checkout", &base_branch]);
    write(repo, "c.txt", "ours\n");
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-am", "ours"],
    );

    // Conflict on c.txt, plus an unrelated staged change.
    let _ = std::process::Command::new("git")
        .current_dir(repo)
        .args(["merge", "feature"])
        .output();
    write(repo, "other.txt", "other\nstaged\n");
    run_git(repo, &["add", "other.txt"]);

    let conflicted_before = run_git_output(repo, &["ls-files", "-u"]);
    assert!(
        conflicted_before.contains("c.txt"),
        "expected c.txt to be unmerged before unstaging:\n{conflicted_before}"
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    opened.unstage(&[]).unwrap();

    let conflicted_after = run_git_output(repo, &["ls-files", "-u"]);
    assert!(
        conflicted_after.contains("c.txt"),
        "unstage-all must leave the conflict in the index:\n{conflicted_after}"
    );
    assert!(
        repo.join(".git").join("MERGE_HEAD").exists(),
        "unstage-all must not abort the merge"
    );

    let status = opened.status().unwrap();
    assert!(
        status
            .unstaged
            .iter()
            .any(|entry| entry.path == PathBuf::from("c.txt") && entry.conflict.is_some()),
        "c.txt must still be reported as conflicted: {:?}",
        status.unstaged
    );
    assert!(
        status.staged.is_empty(),
        "the unrelated staged change must still have been unstaged: {:?}",
        status.staged
    );
}

/// The conflict-safe unstage-all resets named paths rather than everything, so
/// it has to name *both* sides of a staged rename. The status list reports only
/// the destination, and resetting that alone leaves the source path staged as
/// deleted — half a rename in the index.
#[test]
fn unstage_all_during_a_merge_resets_both_sides_of_a_staged_rename() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "c.txt", "base\n");
    // Long enough that rename detection scores the move as a rename.
    write(
        repo,
        "old.txt",
        "alpha\nbravo\ncharlie\ndelta\necho\nfoxtrot\n",
    );
    run_git(repo, &["add", "."]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    let base_branch = run_git_output(repo, &["rev-parse", "--abbrev-ref", "HEAD"]);
    let base_branch = base_branch.trim().to_string();

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "c.txt", "theirs\n");
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-am", "theirs"],
    );
    run_git(repo, &["checkout", &base_branch]);
    write(repo, "c.txt", "ours\n");
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-am", "ours"],
    );

    // Conflict on c.txt, plus a staged rename that has nothing to do with it.
    let _ = std::process::Command::new("git")
        .current_dir(repo)
        .args(["merge", "feature"])
        .output();
    run_git(repo, &["mv", "old.txt", "new.txt"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    opened.unstage(&[]).unwrap();

    let staged = run_git_output(repo, &["diff", "--cached", "--name-only"]);
    assert!(
        !staged.lines().any(|line| line == "old.txt"),
        "unstage-all must not leave old.txt staged as deleted:\n{staged}"
    );
    assert!(
        !staged.lines().any(|line| line == "new.txt"),
        "unstage-all must unstage the rename destination too:\n{staged}"
    );

    // The rename itself stays on disk: unstaging only rewrites the index.
    assert!(
        repo.join("new.txt").exists() && !repo.join("old.txt").exists(),
        "unstage-all must not touch the worktree"
    );

    let conflicted_after = run_git_output(repo, &["ls-files", "-u"]);
    assert!(
        conflicted_after.contains("c.txt"),
        "unstage-all must leave the conflict in the index:\n{conflicted_after}"
    );
    assert!(
        repo.join(".git").join("MERGE_HEAD").exists(),
        "unstage-all must not abort the merge"
    );
}

/// A line-level unstage applies its patch in reverse, so the side it has to
/// match is the index. The patch therefore keeps the additions it is *not*
/// unstaging as context and drops the removals, which the index does not have.
/// Built the staging way instead, git rejects it with "patch does not apply".
#[test]
fn unstage_line_patch_must_describe_the_index_side() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(
        repo,
        "a.txt",
        "context one\nold one\nold two\ncontext two\n",
    );
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    // Stage a two-line modification, then unstage only the first of them.
    write(
        repo,
        "a.txt",
        "context one\nnew one\nnew two\ncontext two\n",
    );
    run_git(repo, &["add", "a.txt"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let staging_shaped = concat!(
        "diff --git a/a.txt b/a.txt\n",
        "--- a/a.txt\n",
        "+++ b/a.txt\n",
        "@@ -1,4 +1,4 @@\n",
        " context one\n",
        " old one\n",
        " old two\n",
        "+new one\n",
        " context two\n",
    );
    assert!(
        opened
            .apply_unified_patch_to_index_with_output(staging_shaped, true)
            .is_err(),
        "a patch describing the HEAD side cannot be reverse-applied to the index"
    );

    let unstage_shaped = concat!(
        "diff --git a/a.txt b/a.txt\n",
        "--- a/a.txt\n",
        "+++ b/a.txt\n",
        "@@ -1,4 +1,4 @@\n",
        " context one\n",
        "+new one\n",
        " new two\n",
        " context two\n",
    );
    opened
        .apply_unified_patch_to_index_with_output(unstage_shaped, true)
        .expect("a patch describing the index side reverse-applies");

    let staged_after = opened
        .diff_unified(&DiffTarget::WorkingTree {
            path: PathBuf::from("a.txt"),
            area: DiffArea::Staged,
        })
        .unwrap();
    assert!(
        staged_after.contains("+new two") && !staged_after.contains("+new one"),
        "only the unstaged line should have left the index:\n{staged_after}"
    );
}

/// A space in a path makes the `diff --git` line ambiguous, so git disambiguates
/// by repeating the name on the `---`/`+++` lines and terminating it with a TAB.
/// Both the diff we hand to the UI and the patch that comes back have to carry
/// that shape for a line-level stage to work at all.
#[test]
fn line_level_staging_round_trips_a_path_containing_spaces() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    let rel = "src/rules - Copy.rs";
    write(repo, rel, "context one\nold one\nold two\ncontext two\n");
    run_git(repo, &["add", rel]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );
    write(repo, rel, "context one\nnew one\nnew two\ncontext two\n");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let unstaged = opened
        .diff_unified(&DiffTarget::WorkingTree {
            path: PathBuf::from(rel),
            area: DiffArea::Unstaged,
        })
        .unwrap();
    assert!(
        unstaged.contains(&format!("+++ b/{rel}\t")),
        "git must repeat the spaced name with a terminating TAB:\n{unstaged}"
    );

    // Stage only the first of the two changed lines, keeping the second's
    // addition out and demoting both removals to context.
    let one_line = format!(
        "diff --git a/{rel} b/{rel}\n\
         --- a/{rel}\t\n\
         +++ b/{rel}\t\n\
         @@ -1,4 +1,4 @@\n\
         \x20context one\n\
         -old one\n\
         \x20old two\n\
         +new one\n\
         \x20context two\n"
    );
    opened
        .apply_unified_patch_to_index_with_output(&one_line, false)
        .expect("a per-line patch for a spaced path must apply to the index");

    let staged_after = opened
        .diff_unified(&DiffTarget::WorkingTree {
            path: PathBuf::from(rel),
            area: DiffArea::Staged,
        })
        .unwrap();
    assert!(
        staged_after.contains("+new one") && !staged_after.contains("+new two"),
        "only the staged line should have reached the index:\n{staged_after}"
    );
}

// ---------------------------------------------------------------------------
// End-to-end conflict resolution workflow tests
// ---------------------------------------------------------------------------

/// End-to-end test: create a merge conflict, load the conflict session,
/// resolve all regions manually, generate resolved text, write it to disk,
/// stage the file, and verify the conflict is fully resolved.
#[test]
fn resolve_conflict_write_and_stage_clears_conflict() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    // Create a BothModified conflict: both sides change the same lines.
    let base_content = "header\nconflict-line\nfooter\n";
    let ours_content = "header\nours-version\nfooter\n";
    let theirs_content = "header\ntheirs-version\nfooter\n";

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "doc.txt", base_content);
    run_git(repo, &["add", "doc.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "doc.txt", theirs_content);
    run_git(repo, &["add", "doc.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "theirs"],
    );

    run_git(repo, &["checkout", "-"]);
    write(repo, "doc.txt", ours_content);
    run_git(repo, &["add", "doc.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "ours"],
    );

    run_git_expect_failure(repo, &["merge", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    // 1. Verify file is in conflict status
    let status = opened.status().unwrap();
    let entry = status
        .unstaged
        .iter()
        .find(|e| e.path == Path::new("doc.txt"))
        .expect("expected conflict entry");
    assert_eq!(entry.kind, FileStatusKind::Conflicted);
    assert_eq!(entry.conflict, Some(FileConflictKind::BothModified));

    // 2. Load conflict session via backend API
    let session = opened
        .conflict_session(Path::new("doc.txt"))
        .unwrap()
        .expect("conflict session");
    assert_eq!(session.strategy, ConflictResolverStrategy::FullTextResolver);
    assert_eq!(session.conflict_kind, FileConflictKind::BothModified);
    let plan = session
        .merge_plan
        .as_ref()
        .expect("full-text Gix session should retain its stage merge plan");
    assert_eq!(session.region_plan_blocks.len(), session.regions.len());
    assert!(
        session
            .region_plan_blocks
            .iter()
            .all(|block_index| plan.blocks.get(*block_index).is_some()),
        "every displayed region should map to a valid plan block",
    );
    let marker_projection = gitcomet_core::merge::render_merge_plan(
        plan,
        &gitcomet_core::merge::MergeOptions {
            style: gitcomet_core::merge::ConflictStyle::Diff3,
            ..Default::default()
        },
    )
    .output;
    let worktree_content = fs::read_to_string(repo.join("doc.txt")).unwrap();
    assert_eq!(session.current_text(), Some(worktree_content.as_str()));
    assert_eq!(
        session.marker_projection_text(),
        Some(marker_projection.as_str())
    );
    assert!(
        marker_projection.contains("|||||||"),
        "stage-backed three-way geometry should include the ancestor section",
    );

    // 3. Verify worktree file contains conflict markers
    let validation = gitcomet_core::services::validate_conflict_resolution_text(&worktree_content);
    assert!(
        validation.has_conflict_markers,
        "worktree file should contain conflict markers"
    );

    // 4. Write manually resolved content (pick ours version)
    let resolved_content = "header\nours-version\nfooter\n";
    let resolved_validation =
        gitcomet_core::services::validate_conflict_resolution_text(resolved_content);
    assert!(
        !resolved_validation.has_conflict_markers,
        "resolved content should have no conflict markers"
    );

    // 5. Write resolved text to worktree and stage
    fs::write(repo.join("doc.txt"), resolved_content).unwrap();
    opened.stage(&[Path::new("doc.txt")]).unwrap();

    // 6. Verify conflict is resolved — no more conflict status
    let status_after = opened.status().unwrap();
    assert!(
        !status_after
            .unstaged
            .iter()
            .any(|e| e.path == Path::new("doc.txt") && e.kind == FileStatusKind::Conflicted),
        "doc.txt should no longer be conflicted after staging resolved content"
    );
}

#[test]
fn resolve_both_added_conflict_write_and_stage_clears_conflict() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    setup_both_added_text_conflict(repo, "new.txt", "ours added\n", "theirs added\n");

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let before = opened.status().unwrap();
    let conflict_entry = before
        .unstaged
        .iter()
        .find(|e| e.path == Path::new("new.txt"))
        .expect("expected both-added conflict path in unstaged status");
    assert_eq!(conflict_entry.kind, FileStatusKind::Conflicted);
    assert_eq!(conflict_entry.conflict, Some(FileConflictKind::BothAdded));

    let merged_before = fs::read_to_string(repo.join("new.txt")).unwrap();
    assert!(
        merged_before.contains("<<<<<<<"),
        "expected merge markers before resolution"
    );

    let session = opened
        .conflict_session(Path::new("new.txt"))
        .unwrap()
        .expect("conflict session for both-added path");
    assert_eq!(session.strategy, ConflictResolverStrategy::FullTextResolver);
    assert_eq!(session.conflict_kind, FileConflictKind::BothAdded);
    assert_eq!(session.total_regions(), 1);
    assert_eq!(session.unsolved_count(), 1);

    let resolved = "resolved both-added\n";
    write(repo, "new.txt", resolved);
    opened.stage(&[Path::new("new.txt")]).unwrap();

    let validation = gitcomet_core::services::validate_conflict_resolution_text(resolved);
    assert!(!validation.has_conflict_markers);
    assert_eq!(validation.marker_lines, 0);

    let after = opened.status().unwrap();
    assert!(
        after
            .unstaged
            .iter()
            .all(|e| e.path != Path::new("new.txt")),
        "expected conflict path to be removed from unstaged after save+stage; status={after:?}"
    );
    assert!(
        after.staged.iter().any(|e| {
            e.path == Path::new("new.txt")
                && matches!(e.kind, FileStatusKind::Modified | FileStatusKind::Added)
        }),
        "expected resolved both-added file to be staged as modified/added; status={after:?}"
    );
    assert_eq!(fs::read_to_string(repo.join("new.txt")).unwrap(), resolved);
}

/// End-to-end test: the stage-backed merge plan materializes trivial changes
/// as automatic context and exposes only genuine conflicts as regions.
#[test]
fn autosolve_safe_resolves_trivial_conflict_regions_end_to_end() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "seed.txt", "seed\n");
    run_git(repo, &["add", "seed.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "seed"],
    );

    // Create a BothModified conflict using synthetic stages.
    // Write a worktree file with conflict markers containing three regions:
    //   Region 0: only ours changed (trivial → OnlyOursChanged)
    //   Region 1: both changed differently (genuine conflict)
    //   Region 2: both sides identical (trivial → IdenticalSides)
    let base_blob = hash_blob(repo, b"base-r0\nbase-r1\nbase-r2\n");
    let ours_blob = hash_blob(repo, b"ours-r0\nours-r1\nsame-r2\n");
    let theirs_blob = hash_blob(repo, b"base-r0\ntheirs-r1\nsame-r2\n");
    set_unmerged_stages(
        repo,
        "multi.txt",
        Some(&base_blob),
        Some(&ours_blob),
        Some(&theirs_blob),
    );

    // Write worktree file with three conflict marker blocks
    let merged_markers = concat!(
        "<<<<<<< HEAD\n",
        "ours-r0\n",
        "||||||| base\n",
        "base-r0\n",
        "=======\n",
        "base-r0\n",
        ">>>>>>> feature\n",
        "<<<<<<< HEAD\n",
        "ours-r1\n",
        "||||||| base\n",
        "base-r1\n",
        "=======\n",
        "theirs-r1\n",
        ">>>>>>> feature\n",
        "<<<<<<< HEAD\n",
        "same-r2\n",
        "||||||| base\n",
        "base-r2\n",
        "=======\n",
        "same-r2\n",
        ">>>>>>> feature\n",
    );
    write(repo, "multi.txt", merged_markers);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    let mut session = opened
        .conflict_session(Path::new("multi.txt"))
        .unwrap()
        .expect("stage-backed conflict session");

    assert_eq!(session.strategy, ConflictResolverStrategy::FullTextResolver);
    assert!(session.merge_plan.is_some());
    assert_eq!(session.total_regions(), 1);
    assert_eq!(
        session.unsolved_count(),
        1,
        "only the genuine conflict should be exposed as a region",
    );
    assert_eq!(session.current_text(), Some(merged_markers));
    let projected = session.marker_projection_text().expect("marker projection");
    assert_eq!(projected.matches("<<<<<<<").count(), 1);
    assert!(projected.contains("ours-r0\n"));
    assert!(projected.contains("same-r2\n"));

    // The plan already resolved the trivial stage changes, so the legacy safe
    // pass has no additional marker region to process.
    let auto_resolved = session.auto_resolve_safe();
    assert_eq!(auto_resolved, 0);
    assert_eq!(session.unsolved_count(), 1);
    assert_eq!(session.next_unresolved_after(0), Some(0));
    assert_eq!(session.prev_unresolved_before(0), Some(0));
}

/// End-to-end test: conflict session for a modify/delete conflict
/// produces correct strategy and payloads, and the "keep" side can be
/// staged to resolve the conflict.
#[test]
fn conflict_session_modify_delete_keep_resolves_conflict() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base content\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    // Feature branch modifies the file
    run_git(repo, &["checkout", "-b", "feature"]);
    write(repo, "a.txt", "modified by feature\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "modify"],
    );

    // Main branch deletes the file
    run_git(repo, &["checkout", "-"]);
    run_git(repo, &["rm", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "delete"],
    );

    run_git_expect_failure(repo, &["merge", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    // Verify conflict session for modify/delete
    let session = opened
        .conflict_session(Path::new("a.txt"))
        .unwrap()
        .expect("conflict session for modify/delete");
    assert_eq!(
        session.strategy,
        ConflictResolverStrategy::TwoWayKeepDelete,
        "modify/delete conflicts should use TwoWayKeepDelete strategy"
    );
    assert_eq!(session.conflict_kind, FileConflictKind::DeletedByUs);

    // Ours deleted (absent), theirs has content
    assert!(
        session.ours.is_absent(),
        "ours (delete side) should be absent"
    );
    assert!(
        session.theirs.as_text().is_some(),
        "theirs (modify side) should have text"
    );
    assert_eq!(
        session.unsolved_count(),
        1,
        "two-way non-marker conflict sessions should expose one unresolved decision region"
    );
    assert_eq!(session.regions[0].ours, "");
    assert_eq!(session.regions[0].theirs, "modified by feature\n");

    // Resolve by keeping theirs (the modified version)
    opened
        .checkout_conflict_side(Path::new("a.txt"), ConflictSide::Theirs)
        .unwrap();

    // Verify file is restored and no longer conflicted
    assert_eq!(
        fs::read_to_string(repo.join("a.txt")).unwrap(),
        "modified by feature\n"
    );
    let status = opened.status().unwrap();
    assert!(
        !status
            .unstaged
            .iter()
            .any(|e| e.path == Path::new("a.txt") && e.kind == FileStatusKind::Conflicted),
        "a.txt should no longer be conflicted after keeping theirs"
    );
}

/// Validates the safety gate: `validate_conflict_resolution_text` correctly
/// detects remaining markers in partially-resolved text.
#[test]
fn validate_conflict_resolution_detects_partial_resolution() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    use gitcomet_core::services::validate_conflict_resolution_text;

    // Fully resolved text — no markers
    let clean = "line1\nline2\nline3\n";
    assert!(!validate_conflict_resolution_text(clean).has_conflict_markers);

    // Partially resolved — one conflict block remains
    let partial = concat!(
        "resolved section\n",
        "<<<<<<< HEAD\n",
        "ours\n",
        "=======\n",
        "theirs\n",
        ">>>>>>> feature\n",
        "another resolved section\n",
    );
    let v = validate_conflict_resolution_text(partial);
    assert!(v.has_conflict_markers);
    assert_eq!(v.marker_lines, 3); // <<<<<<<, =======, >>>>>>>

    // diff3-style markers
    let diff3 = concat!(
        "<<<<<<< HEAD\n",
        "ours\n",
        "||||||| base\n",
        "base\n",
        "=======\n",
        "theirs\n",
        ">>>>>>> feature\n",
    );
    let v3 = validate_conflict_resolution_text(diff3);
    assert!(v3.has_conflict_markers);
    assert_eq!(v3.marker_lines, 4); // <<<<<<<, |||||||, =======, >>>>>>>
}

/// End-to-end test: BothDeleted text conflict session uses DecisionOnly
/// strategy, and restoring from base via `checkout_conflict_side(Base)`
/// resolves the conflict.
#[test]
fn conflict_session_both_deleted_restore_from_base_resolves_conflict() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "seed.txt", "seed\n");
    run_git(repo, &["add", "seed.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "seed"],
    );

    // BothDeleted: only base stage present, no ours or theirs
    let base_blob = hash_blob(repo, b"original content\n");
    set_unmerged_stages(repo, "removed.txt", Some(base_blob.as_str()), None, None);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    // Verify conflict session
    let session = opened
        .conflict_session(Path::new("removed.txt"))
        .unwrap()
        .expect("conflict session for BothDeleted");
    assert_eq!(session.conflict_kind, FileConflictKind::BothDeleted);
    assert_eq!(session.strategy, ConflictResolverStrategy::DecisionOnly);
    assert!(
        matches!(session.base, ConflictPayload::Text(ref t) if t.as_ref() == "original content\n")
    );
    assert!(session.ours.is_absent());
    assert!(session.theirs.is_absent());
    assert!(matches!(
        session.current.as_ref(),
        Some(ConflictPayload::Absent)
    ));
    assert_eq!(session.unsolved_count(), 1);

    // Resolve by accepting deletion
    opened
        .accept_conflict_deletion(Path::new("removed.txt"))
        .unwrap();

    // Verify conflict is resolved
    let status = opened.status().unwrap();
    assert!(
        !status
            .unstaged
            .iter()
            .any(|e| e.path == Path::new("removed.txt") && e.kind == FileStatusKind::Conflicted),
        "removed.txt should no longer be conflicted after accepting deletion"
    );
    assert!(
        !repo.join("removed.txt").exists(),
        "file should be deleted after accepting deletion"
    );
}

/// End-to-end test: AddedByUs conflict session uses TwoWayKeepDelete
/// strategy, and keeping the file via `checkout_conflict_side(Ours)`
/// resolves the conflict.
#[test]
fn conflict_session_added_by_us_keep_resolves_conflict() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "seed.txt", "seed\n");
    run_git(repo, &["add", "seed.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "seed"],
    );

    // AddedByUs: only ours stage present (no base, no theirs)
    let ours_blob = hash_blob(repo, b"added by us\n");
    set_unmerged_stages(repo, "new.txt", None, Some(ours_blob.as_str()), None);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    // Verify status
    let status = opened.status().unwrap();
    let entry = status
        .unstaged
        .iter()
        .find(|e| e.path == Path::new("new.txt"))
        .expect("expected AddedByUs conflict entry");
    assert_eq!(entry.kind, FileStatusKind::Conflicted);
    assert_eq!(entry.conflict, Some(FileConflictKind::AddedByUs));

    // Verify conflict session
    let session = opened
        .conflict_session(Path::new("new.txt"))
        .unwrap()
        .expect("conflict session for AddedByUs");
    assert_eq!(session.conflict_kind, FileConflictKind::AddedByUs);
    assert_eq!(session.strategy, ConflictResolverStrategy::TwoWayKeepDelete);
    assert!(session.base.is_absent());
    assert!(matches!(session.ours, ConflictPayload::Text(ref t) if t.as_ref() == "added by us\n"));
    assert!(session.theirs.is_absent());
    assert!(matches!(
        session.current.as_ref(),
        Some(ConflictPayload::Absent)
    ));
    assert_eq!(session.unsolved_count(), 1);

    // Resolve by keeping ours (the added file)
    opened
        .checkout_conflict_side(Path::new("new.txt"), ConflictSide::Ours)
        .unwrap();

    // Verify file exists and conflict is resolved
    assert_eq!(
        fs::read_to_string(repo.join("new.txt")).unwrap(),
        "added by us\n"
    );
    let status_after = opened.status().unwrap();
    assert!(
        !status_after
            .unstaged
            .iter()
            .any(|e| e.path == Path::new("new.txt") && e.kind == FileStatusKind::Conflicted),
        "new.txt should no longer be conflicted after keeping ours"
    );
    assert!(
        status_after
            .staged
            .iter()
            .any(|e| e.path == Path::new("new.txt")),
        "new.txt should be staged after resolution"
    );
}

/// End-to-end test: AddedByThem conflict session uses TwoWayKeepDelete
/// strategy, and keeping the file via `checkout_conflict_side(Theirs)`
/// resolves the conflict.
#[test]
fn conflict_session_added_by_them_keep_resolves_conflict() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "seed.txt", "seed\n");
    run_git(repo, &["add", "seed.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "seed"],
    );

    // AddedByThem: only theirs stage present (no base, no ours)
    let theirs_blob = hash_blob(repo, b"added by them\n");
    set_unmerged_stages(
        repo,
        "their_new.txt",
        None,
        None,
        Some(theirs_blob.as_str()),
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    // Verify status
    let status = opened.status().unwrap();
    let entry = status
        .unstaged
        .iter()
        .find(|e| e.path == Path::new("their_new.txt"))
        .expect("expected AddedByThem conflict entry");
    assert_eq!(entry.kind, FileStatusKind::Conflicted);
    assert_eq!(entry.conflict, Some(FileConflictKind::AddedByThem));

    // Verify conflict session
    let session = opened
        .conflict_session(Path::new("their_new.txt"))
        .unwrap()
        .expect("conflict session for AddedByThem");
    assert_eq!(session.conflict_kind, FileConflictKind::AddedByThem);
    assert_eq!(session.strategy, ConflictResolverStrategy::TwoWayKeepDelete);
    assert!(session.base.is_absent());
    assert!(session.ours.is_absent());
    assert!(
        matches!(session.theirs, ConflictPayload::Text(ref t) if t.as_ref() == "added by them\n")
    );
    assert!(matches!(
        session.current.as_ref(),
        Some(ConflictPayload::Absent)
    ));
    assert_eq!(session.unsolved_count(), 1);

    // Resolve by keeping theirs (the added file)
    opened
        .checkout_conflict_side(Path::new("their_new.txt"), ConflictSide::Theirs)
        .unwrap();

    // Verify file exists and conflict is resolved
    assert_eq!(
        fs::read_to_string(repo.join("their_new.txt")).unwrap(),
        "added by them\n"
    );
    let status_after = opened.status().unwrap();
    assert!(
        !status_after
            .unstaged
            .iter()
            .any(|e| e.path == Path::new("their_new.txt") && e.kind == FileStatusKind::Conflicted),
        "their_new.txt should no longer be conflicted after keeping theirs"
    );
    assert!(
        status_after
            .staged
            .iter()
            .any(|e| e.path == Path::new("their_new.txt")),
        "their_new.txt should be staged after resolution"
    );
}

/// End-to-end test: DeletedByThem conflict session uses TwoWayKeepDelete
/// strategy (base+ours present, theirs absent), and keeping ours
/// via `checkout_conflict_side(Ours)` resolves the conflict.
#[test]
fn conflict_session_deleted_by_them_keep_ours_resolves_conflict() {
    if !require_git_shell_for_status_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    write(repo, "a.txt", "base content\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    // Feature branch deletes the file
    run_git(repo, &["checkout", "-b", "feature"]);
    run_git(repo, &["rm", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "delete"],
    );

    // Main branch modifies the file
    run_git(repo, &["checkout", "-"]);
    write(repo, "a.txt", "modified by us\n");
    run_git(repo, &["add", "a.txt"]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "modify"],
    );

    run_git_expect_failure(repo, &["merge", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    // Verify status shows DeletedByThem
    let status = opened.status().unwrap();
    let entry = status
        .unstaged
        .iter()
        .find(|e| e.path == Path::new("a.txt") && e.kind == FileStatusKind::Conflicted)
        .expect("expected DeletedByThem conflict entry");
    assert_eq!(entry.conflict, Some(FileConflictKind::DeletedByThem));

    // Verify conflict session
    let session = opened
        .conflict_session(Path::new("a.txt"))
        .unwrap()
        .expect("conflict session for DeletedByThem");
    assert_eq!(session.conflict_kind, FileConflictKind::DeletedByThem);
    assert_eq!(session.strategy, ConflictResolverStrategy::TwoWayKeepDelete);
    assert!(session.base.as_text().is_some());
    assert!(
        matches!(session.ours, ConflictPayload::Text(ref t) if t.as_ref() == "modified by us\n"),
        "ours (modified side) should have text"
    );
    assert!(
        session.theirs.is_absent(),
        "theirs (delete side) should be absent"
    );
    assert_eq!(session.unsolved_count(), 1);
    assert_eq!(session.regions[0].ours, "modified by us\n");
    assert_eq!(session.regions[0].theirs, "");

    // Resolve by keeping ours (the modified version)
    opened
        .checkout_conflict_side(Path::new("a.txt"), ConflictSide::Ours)
        .unwrap();

    // Verify file is kept and conflict is resolved
    assert_eq!(
        fs::read_to_string(repo.join("a.txt")).unwrap(),
        "modified by us\n"
    );
    let status_after = opened.status().unwrap();
    assert!(
        !status_after
            .unstaged
            .iter()
            .any(|e| e.path == Path::new("a.txt") && e.kind == FileStatusKind::Conflicted),
        "a.txt should no longer be conflicted after keeping ours"
    );
}
