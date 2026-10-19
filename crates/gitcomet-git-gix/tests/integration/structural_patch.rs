use gitcomet_core::domain::{DiffArea, DiffTarget};
use gitcomet_core::patch_action::{PatchOperation, SourceLineSelection};
use gitcomet_core::services::{CancellationToken, GitBackend, GitRepository};
use gitcomet_git_gix::GixBackend;
use std::collections::BTreeSet;
use std::path::Path;

use super::test_git_env;

fn git(path: &Path, args: &[&str]) -> Vec<u8> {
    let mut command = gitcomet_core::test_support::git_fixture::git_command();
    test_git_env::apply(&mut command);
    let output = command.arg("-C").arg(path).args(args).output().unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn fixture(old: &str, new: &str) -> (tempfile::TempDir, std::sync::Arc<dyn GitRepository>) {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-b", "main"]);
    git(dir.path(), &["config", "user.email", "fixture@example.com"]);
    git(dir.path(), &["config", "user.name", "Fixture"]);
    git(dir.path(), &["config", "core.autocrlf", "false"]);
    std::fs::write(dir.path().join("file.rs"), old).unwrap();
    git(dir.path(), &["add", "file.rs"]);
    git(
        dir.path(),
        &["-c", "commit.gpgsign=false", "commit", "-qm", "base"],
    );
    std::fs::write(dir.path().join("file.rs"), new).unwrap();
    let repo = GixBackend.open(dir.path()).unwrap();
    (dir, repo)
}

fn prepare(
    repo: &dyn GitRepository,
    operation: PatchOperation,
    old: &[u32],
    new: &[u32],
) -> gitcomet_core::patch_action::PreparedDiffPatch {
    let target = DiffTarget::working_tree("file.rs".into(), operation.area());
    let token = CancellationToken::new();
    let snapshot = repo.diff_action_snapshot(&target, &token).unwrap();
    repo.prepare_diff_patch(
        &target,
        operation,
        &SourceLineSelection {
            old: BTreeSet::from_iter(old.iter().copied()),
            new: BTreeSet::from_iter(new.iter().copied()),
        },
        &snapshot.old,
        &snapshot.new,
        &token,
    )
    .unwrap()
}

#[test]
fn selected_lines_round_trip_through_index_and_worktree_without_neighbours() {
    let old = "fn a() { 1 }\nfn b() { 2 }\n";
    let new = "fn a() { 10 }\nfn b() { 20 }\n";
    let (dir, repo) = fixture(old, new);
    let staged = "fn a() { 10 }\nfn b() { 2 }\n";
    let patch = prepare(repo.as_ref(), PatchOperation::Stage, &[1], &[1]);
    repo.apply_prepared_diff_patch(&patch).unwrap();
    assert_eq!(git(dir.path(), &["show", ":file.rs"]), staged.as_bytes());
    assert_eq!(
        std::fs::read(dir.path().join("file.rs")).unwrap(),
        new.as_bytes()
    );
    let patch = prepare(repo.as_ref(), PatchOperation::Unstage, &[1], &[1]);
    repo.apply_prepared_diff_patch(&patch).unwrap();
    assert_eq!(git(dir.path(), &["show", ":file.rs"]), old.as_bytes());
    let patch = prepare(repo.as_ref(), PatchOperation::Discard, &[1], &[1]);
    repo.apply_prepared_diff_patch(&patch).unwrap();
    assert_eq!(
        std::fs::read(dir.path().join("file.rs")).unwrap(),
        b"fn a() { 1 }\nfn b() { 20 }\n"
    );
    assert_eq!(git(dir.path(), &["show", ":file.rs"]), old.as_bytes());
}

#[test]
fn preview_rejects_source_and_attribute_changes_without_writing() {
    let (dir, repo) = fixture("old\nkeep\n", "new\nkeep\n");
    let patch = prepare(repo.as_ref(), PatchOperation::Stage, &[1], &[1]);
    std::fs::write(dir.path().join("file.rs"), "new\nchanged elsewhere\n").unwrap();
    assert!(repo.apply_prepared_diff_patch(&patch).is_err());
    assert_eq!(git(dir.path(), &["show", ":file.rs"]), b"old\nkeep\n");
    let patch = prepare(repo.as_ref(), PatchOperation::Discard, &[1], &[1]);
    std::fs::write(dir.path().join(".gitattributes"), "file.rs -text\n").unwrap();
    assert!(repo.apply_prepared_diff_patch(&patch).is_err());
    assert_eq!(
        std::fs::read(dir.path().join("file.rs")).unwrap(),
        b"new\nchanged elsewhere\n"
    );
    let patch = prepare(repo.as_ref(), PatchOperation::Stage, &[1], &[1]);
    repo.apply_prepared_diff_patch(&patch).unwrap();
    assert_eq!(git(dir.path(), &["show", ":file.rs"]), b"new\nkeep\n");
}

#[test]
fn exact_patch_preserves_crlf_and_missing_final_newline() {
    for (old, new) in [("old\r\nkeep\r\n", "new\r\nkeep\r\n"), ("old", "new")] {
        let (dir, repo) = fixture(old, new);
        let patch = prepare(repo.as_ref(), PatchOperation::Stage, &[1], &[1]);
        repo.apply_prepared_diff_patch(&patch).unwrap();
        assert_eq!(git(dir.path(), &["show", ":file.rs"]), new.as_bytes());
        let patch = prepare(repo.as_ref(), PatchOperation::Unstage, &[1], &[1]);
        repo.apply_prepared_diff_patch(&patch).unwrap();
        assert_eq!(git(dir.path(), &["show", ":file.rs"]), old.as_bytes());
    }
}

#[test]
fn one_side_and_separate_hunks_keep_unselected_changes_out() {
    let old = "old\n1\n2\n3\n4\n5\n6\n7\n8\nold tail\n";
    let new = "new\n1\n2\n3\n4\n5\n6\n7\n8\nnew tail\n";
    let (dir, repo) = fixture(old, new);
    let patch = prepare(repo.as_ref(), PatchOperation::Stage, &[], &[1, 10]);
    repo.apply_prepared_diff_patch(&patch).unwrap();
    assert_eq!(
        git(dir.path(), &["show", ":file.rs"]),
        b"old\nnew\n1\n2\n3\n4\n5\n6\n7\n8\nold tail\nnew tail\n"
    );
    assert_eq!(
        std::fs::read(dir.path().join("file.rs")).unwrap(),
        new.as_bytes()
    );
}

#[test]
fn mismatched_display_cannot_prepare_a_patch() {
    let (dir, repo) = fixture("old\n", "new\n");
    let result = repo.prepare_diff_patch(
        &DiffTarget::working_tree("file.rs".into(), DiffArea::Unstaged),
        PatchOperation::Stage,
        &SourceLineSelection {
            old: BTreeSet::from([1]),
            new: BTreeSet::from([1]),
        },
        "old\n",
        "stale display\n",
        &CancellationToken::new(),
    );
    assert!(result.is_err());
    assert_eq!(git(dir.path(), &["show", ":file.rs"]), b"old\n");
    assert_eq!(std::fs::read(dir.path().join("file.rs")).unwrap(), b"new\n");
}

#[test]
fn untracked_partial_stage_and_discard_preserve_remaining_lines() {
    let (dir, repo) = fixture("base\n", "base\n");
    git(dir.path(), &["config", "color.ui", "always"]);
    git(dir.path(), &["rm", "--cached", "file.rs"]);
    git(
        dir.path(),
        &["-c", "commit.gpgsign=false", "commit", "-qm", "remove"],
    );
    std::fs::write(dir.path().join("file.rs"), "one\ntwo\n").unwrap();
    let patch = prepare(repo.as_ref(), PatchOperation::Stage, &[], &[1]);
    repo.apply_prepared_diff_patch(&patch).unwrap();
    assert_eq!(git(dir.path(), &["show", ":file.rs"]), b"one\n");
    let patch = prepare(repo.as_ref(), PatchOperation::Unstage, &[], &[1]);
    repo.apply_prepared_diff_patch(&patch).unwrap();
    assert!(git(dir.path(), &["ls-files", "--", "file.rs"]).is_empty());
    let patch = prepare(repo.as_ref(), PatchOperation::Discard, &[], &[1]);
    repo.apply_prepared_diff_patch(&patch).unwrap();
    assert_eq!(std::fs::read(dir.path().join("file.rs")).unwrap(), b"two\n");
}

#[test]
fn partial_deletion_stages_content_without_deleting_retained_lines() {
    let (dir, repo) = fixture("one\ntwo\n", "one\ntwo\n");
    std::fs::remove_file(dir.path().join("file.rs")).unwrap();
    let patch = prepare(repo.as_ref(), PatchOperation::Stage, &[1], &[]);
    repo.apply_prepared_diff_patch(&patch).unwrap();
    assert_eq!(git(dir.path(), &["show", ":file.rs"]), b"two\n");
    assert!(!dir.path().join("file.rs").exists());
}

#[test]
fn partial_unstage_of_added_file_keeps_the_file_and_unselected_lines() {
    let (dir, repo) = fixture("base\n", "base\n");
    git(dir.path(), &["rm", "file.rs"]);
    git(
        dir.path(),
        &["-c", "commit.gpgsign=false", "commit", "-qm", "remove"],
    );
    std::fs::write(dir.path().join("file.rs"), "one\ntwo\n").unwrap();
    git(dir.path(), &["add", "file.rs"]);
    let patch = prepare(repo.as_ref(), PatchOperation::Unstage, &[], &[1]);
    repo.apply_prepared_diff_patch(&patch).unwrap();
    assert_eq!(git(dir.path(), &["show", ":file.rs"]), b"two\n");
    assert_eq!(
        std::fs::read(dir.path().join("file.rs")).unwrap(),
        b"one\ntwo\n"
    );
}

#[cfg(unix)]
#[test]
fn content_selection_does_not_stage_a_mode_change() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, repo) = fixture("old\n", "new\n");
    git(dir.path(), &["config", "core.filemode", "true"]);
    std::fs::set_permissions(
        dir.path().join("file.rs"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let patch = prepare(repo.as_ref(), PatchOperation::Stage, &[1], &[1]);
    repo.apply_prepared_diff_patch(&patch).unwrap();
    assert!(git(dir.path(), &["ls-files", "--stage", "file.rs"]).starts_with(b"100644 "));
    assert_eq!(git(dir.path(), &["show", ":file.rs"]), b"new\n");
}

#[test]
fn linked_target_cannot_mutate_the_main_worktree_with_matching_sources() {
    let (dir, repo) = fixture("old\n", "new\n");
    let linked = dir.path().join("linked");
    git(
        dir.path(),
        &["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
    );
    std::fs::write(linked.join("file.rs"), "new\n").unwrap();
    let target =
        DiffTarget::working_tree("file.rs".into(), DiffArea::Unstaged).in_worktree(linked.clone());
    let result = repo.prepare_diff_patch(
        &target,
        PatchOperation::Stage,
        &SourceLineSelection {
            old: BTreeSet::from([1]),
            new: BTreeSet::from([1]),
        },
        "old\n",
        "new\n",
        &CancellationToken::new(),
    );
    assert!(result.is_err());
    assert_eq!(git(dir.path(), &["show", ":file.rs"]), b"old\n");
    assert_eq!(git(&linked, &["show", ":file.rs"]), b"old\n");
    let patch = prepare(repo.as_ref(), PatchOperation::Stage, &[1], &[1]);
    repo.apply_prepared_diff_patch(&patch).unwrap();
    assert_eq!(git(dir.path(), &["show", ":file.rs"]), b"new\n");
    assert_eq!(git(&linked, &["show", ":file.rs"]), b"old\n");
}

#[cfg(unix)]
#[test]
fn pathspec_characters_address_only_the_literal_file() {
    let (dir, repo) = fixture("old\n", "old\n");
    for name in ["file*.rs", "fileX.rs"] {
        std::fs::write(dir.path().join(name), "old\n").unwrap();
    }
    git(dir.path(), &["add", "."]);
    git(
        dir.path(),
        &["-c", "commit.gpgsign=false", "commit", "-qm", "paths"],
    );
    for name in ["file*.rs", "fileX.rs"] {
        std::fs::write(dir.path().join(name), "new\n").unwrap();
    }
    let target = DiffTarget::working_tree("file*.rs".into(), DiffArea::Unstaged);
    let patch = repo
        .prepare_diff_patch(
            &target,
            PatchOperation::Stage,
            &SourceLineSelection {
                old: BTreeSet::from([1]),
                new: BTreeSet::from([1]),
            },
            "old\n",
            "new\n",
            &CancellationToken::new(),
        )
        .unwrap();
    repo.apply_prepared_diff_patch(&patch).unwrap();
    assert_eq!(git(dir.path(), &["show", ":file*.rs"]), b"new\n");
    assert_eq!(git(dir.path(), &["show", ":fileX.rs"]), b"old\n");
}

/// The exact patch came from porcelain `git diff`, which still colours under
/// `color.ui=false` when `color.diff=always`, and prints a blank context line
/// as an empty one under `diff.suppressBlankEmpty`: either patch maps to no
/// source line, so every partial action failed for that user.
#[test]
fn user_diff_config_does_not_reshape_the_exact_patch() {
    let old = "fn a() {\n\n    1\n}\n";
    let new = "fn a() {\n\n    2\n}\n";
    for (key, value) in [
        ("color.diff", "always"),
        ("diff.suppressBlankEmpty", "true"),
        ("diff.context", "0"),
        ("diff.noprefix", "true"),
        ("diff.relative", "true"),
    ] {
        let (dir, repo) = fixture(old, new);
        git(dir.path(), &["config", key, value]);
        let patch = prepare(repo.as_ref(), PatchOperation::Stage, &[3], &[3]);
        repo.apply_prepared_diff_patch(&patch).unwrap();
        assert_eq!(
            git(dir.path(), &["show", ":file.rs"]),
            new.as_bytes(),
            "{key}"
        );
    }
}

/// Unstaging reads HEAD and the index only, yet the snapshot also held the
/// worktree copy: an editor saving between prepare and apply made it stale.
#[test]
fn unstage_survives_a_worktree_edit_before_it_applies() {
    let (dir, repo) = fixture("old\nkeep\n", "new\nkeep\n");
    git(dir.path(), &["add", "file.rs"]);
    let patch = prepare(repo.as_ref(), PatchOperation::Unstage, &[1], &[1]);
    std::fs::write(dir.path().join("file.rs"), "autosaved\nkeep\n").unwrap();
    repo.apply_prepared_diff_patch(&patch).unwrap();
    assert_eq!(git(dir.path(), &["show", ":file.rs"]), b"old\nkeep\n");
    assert_eq!(
        std::fs::read(dir.path().join("file.rs")).unwrap(),
        b"autosaved\nkeep\n"
    );
}

/// Unstaging never reads the worktree, so its copy growing past the source
/// limit must not refuse it.
#[test]
fn unstage_ignores_an_oversized_worktree_copy() {
    let (dir, repo) = fixture("old\nkeep\n", "new\nkeep\n");
    git(dir.path(), &["add", "file.rs"]);
    let oversized = "x\n".repeat(gitcomet_core::patch_action::SOURCE_BYTE_LIMIT / 2 + 1);
    std::fs::write(dir.path().join("file.rs"), &oversized).unwrap();
    let patch = prepare(repo.as_ref(), PatchOperation::Unstage, &[1], &[1]);
    repo.apply_prepared_diff_patch(&patch).unwrap();
    assert_eq!(git(dir.path(), &["show", ":file.rs"]), b"old\nkeep\n");
}
