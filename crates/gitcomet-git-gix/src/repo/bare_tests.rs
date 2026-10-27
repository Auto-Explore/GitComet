use super::*;
use crate::test_support::{git_stdout, git_success, init_test_repo, open_repo};
use gitcomet_core::services::{GitBackend, GitRepository};
use std::fs;

pub(super) fn bare_fixture() -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let checkout = directory.path().join("checkout");
    fs::create_dir(&checkout).unwrap();
    init_test_repo(&checkout);
    fs::write(checkout.join("tracked.txt"), "original\n").unwrap();
    git_success(&checkout, &["add", "."]);
    git_success(&checkout, &["commit", "-m", "initial"]);
    let bare = directory.path().join("history.git");
    git_success(
        directory.path(),
        &[
            "clone",
            "--bare",
            checkout.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
    );
    (directory, bare)
}

#[test]
fn native_mode_classifies_bare_normal_unborn_and_dot_git_named_checkout() {
    let (_directory, bare) = bare_fixture();
    assert_eq!(
        open_repo(&bare).repository_mode().unwrap(),
        RepositoryMode::Bare
    );
    let directory = tempfile::tempdir().unwrap();
    let checkout = directory.path().join("checkout.git");
    fs::create_dir(&checkout).unwrap();
    init_test_repo(&checkout);
    assert_eq!(
        open_repo(&checkout).repository_mode().unwrap(),
        RepositoryMode::WorkingTree
    );
    let unborn = directory.path().join("unborn.git");
    git_success(
        directory.path(),
        &["init", "--bare", unborn.to_str().unwrap()],
    );
    assert_eq!(
        open_repo(&unborn).repository_mode().unwrap(),
        RepositoryMode::Bare
    );
}

#[test]
fn native_mode_is_silent_and_keeps_attached_operation_cancellation() {
    use gitcomet_core::git_operation::{self, GitOperationContext};

    let (directory, bare) = bare_fixture();
    let (sender, receiver) = std::sync::mpsc::channel();
    let operation = GitOperationContext::new("native mode", move |_, event| {
        sender.send(event).unwrap();
    });
    let _scope = git_operation::attach(&operation);
    assert_eq!(native_mode(&bare).unwrap(), RepositoryMode::Bare);
    assert_eq!(
        native_mode(&directory.path().join("checkout")).unwrap(),
        RepositoryMode::WorkingTree
    );
    assert!(receiver.try_iter().next().is_none());
    assert!(git_operation::cancel(operation.id()));
    assert!(matches!(
        native_mode(&bare).unwrap_err().kind(),
        ErrorKind::Cancelled
    ));
}

#[test]
fn linked_worktree_is_normal_with_shared_core_bare_true() {
    let (_directory, bare) = bare_fixture();
    let linked = bare.parent().unwrap().join("linked");
    git_success(
        &bare,
        &["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
    );
    assert_eq!(git_stdout(&bare, &["config", "core.bare"]), "true");
    let repo = open_repo(&linked);
    assert_eq!(repo.repository_mode().unwrap(), RepositoryMode::WorkingTree);
    let listed = repo.list_worktrees(&CancellationToken::new()).unwrap();
    assert!(
        listed
            .iter()
            .any(|worktree| worktree.path == bare && worktree.bare)
    );
    assert!(
        listed
            .iter()
            .any(|worktree| worktree.path == linked && !worktree.bare)
    );
}

#[test]
fn config_repair_preserves_dirty_staged_untracked_and_index_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    init_test_repo(root);
    fs::write(root.join("tracked.txt"), "initial\n").unwrap();
    git_success(root, &["add", "."]);
    git_success(root, &["commit", "-m", "initial"]);
    fs::write(root.join("tracked.txt"), "staged\n").unwrap();
    git_success(root, &["add", "."]);
    fs::write(root.join("tracked.txt"), "dirty\n").unwrap();
    fs::write(root.join("untracked.txt"), "untouched\n").unwrap();
    let before_index = fs::read(root.join(".git/index")).unwrap();
    let before_head = fs::read(root.join(".git/HEAD")).unwrap();
    let before_status = git_stdout(root, &["status", "--porcelain=v1"]);
    git_success(root, &["config", "core.bare", "true"]);
    let repo = open_repo(root);
    let preview = repo.preview_disable_bare_mode().unwrap();
    assert_eq!(preview.layout, BareConversionLayout::ExistingCheckout);
    repo.disable_bare_mode(&preview).unwrap();
    assert_eq!(fs::read(root.join(".git/index")).unwrap(), before_index);
    assert_eq!(fs::read(root.join(".git/HEAD")).unwrap(), before_head);
    assert_eq!(
        fs::read_to_string(root.join("tracked.txt")).unwrap(),
        "dirty\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("untracked.txt")).unwrap(),
        "untouched\n"
    );
    assert_eq!(
        git_stdout(root, &["status", "--porcelain=v1"]),
        before_status
    );
    assert!(matches!(
        repo.repository_mode().unwrap_err().kind(),
        ErrorKind::RepositoryChanged
    ));
    assert_eq!(
        crate::GixBackend
            .open(root)
            .unwrap()
            .repository_mode()
            .unwrap(),
        RepositoryMode::WorkingTree
    );
}

#[cfg(test)]
#[test]
fn genuine_bare_conversion_installs_checkout_in_selected_folder() {
    let (_directory, bare) = bare_fixture();
    let old_head = fs::read(bare.join("HEAD")).unwrap();
    let repo = crate::GixBackend.open(&bare).unwrap();
    let preview = repo.preview_disable_bare_mode().unwrap();
    assert_eq!(preview.layout, BareConversionLayout::BareDirectory);
    let result = repo.disable_bare_mode(&preview).unwrap();
    assert_eq!(result.root, bare);
    assert_eq!(result.old_common_dir, bare);
    assert_eq!(result.new_common_dir, bare.join(".git"));
    assert_eq!(fs::read(bare.join(".git/HEAD")).unwrap(), old_head);
    assert_eq!(
        fs::read_to_string(bare.join("tracked.txt")).unwrap(),
        "original\n"
    );
    assert_eq!(git_stdout(&bare, &["status", "--porcelain=v1"]), "");
    assert!(matches!(
        repo.repository_mode().unwrap_err().kind(),
        ErrorKind::RepositoryChanged
    ));
    assert_eq!(
        crate::GixBackend
            .open(&bare)
            .unwrap()
            .repository_mode()
            .unwrap(),
        RepositoryMode::WorkingTree
    );
}

#[test]
fn unborn_bare_conversion_preserves_symbolic_head() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("unborn");
    git_success(
        directory.path(),
        &[
            "init",
            "--bare",
            "--initial-branch=unborn",
            root.to_str().unwrap(),
        ],
    );
    let repo = open_repo(&root);
    let preview = repo.preview_disable_bare_mode().unwrap();
    assert_eq!(
        preview.head,
        BareConversionHead::Branch {
            name: "unborn".into(),
            commit: None
        }
    );
    repo.disable_bare_mode(&preview).unwrap();
    assert_eq!(
        git_stdout(&root, &["symbolic-ref", "HEAD"]),
        "refs/heads/unborn"
    );
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
}

#[test]
fn head_branch_in_linked_checkout_refuses_without_changing_source() {
    let (_directory, bare) = bare_fixture();
    let branch = git_stdout(&bare, &["symbolic-ref", "--short", "HEAD"]);
    let linked = bare.parent().unwrap().join("linked");
    git_success(
        &bare,
        &["worktree", "add", linked.to_str().unwrap(), &branch],
    );
    let before = storage::tree_digest(&bare).unwrap();
    let error = open_repo(&bare).preview_disable_bare_mode().unwrap_err();
    assert!(matches!(
        error.kind(),
        ErrorKind::BareConversionRefused(BareConversionRefusal::BranchInUse { .. })
    ));
    assert_eq!(storage::tree_digest(&bare).unwrap(), before);
}

#[test]
fn stale_preview_rejects_ref_changes_without_mutation() {
    let (_directory, bare) = bare_fixture();
    let repo = open_repo(&bare);
    let preview = repo.preview_disable_bare_mode().unwrap();
    git_success(&bare, &["branch", "after-preview"]);
    let before = storage::tree_digest(&bare).unwrap();
    let error = repo.disable_bare_mode(&preview).unwrap_err();
    assert!(matches!(
        error.kind(),
        ErrorKind::BareConversionRefused(BareConversionRefusal::StalePreview)
    ));
    assert_eq!(storage::tree_digest(&bare).unwrap(), before);
    assert!(!bare.join(".git").exists());
}

#[test]
fn relocation_repairs_linked_worktrees_without_changing_their_files() {
    let (_directory, bare) = bare_fixture();
    let linked = bare.parent().unwrap().join("linked");
    git_success(
        &bare,
        &["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
    );
    fs::write(linked.join("tracked.txt"), "linked dirty\n").unwrap();
    let repo = open_repo(&bare);
    let preview = repo.preview_disable_bare_mode().unwrap();
    let result = repo.disable_bare_mode(&preview).unwrap();
    assert!(result.affected_roots.contains(&linked));
    assert_eq!(
        git_stdout(&linked, &["rev-parse", "--git-common-dir"]),
        bare.join(".git").to_str().unwrap()
    );
    assert_eq!(
        fs::read_to_string(linked.join("tracked.txt")).unwrap(),
        "linked dirty\n"
    );
    assert_eq!(git_stdout(&linked, &["branch", "--show-current"]), "linked");
}

#[test]
fn watcher_metadata_tracks_native_linked_and_converted_worktrees() {
    let (_directory, bare) = bare_fixture();
    let linked = bare.parent().unwrap().join("linked");
    git_success(
        &bare,
        &["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
    );
    let before = crate::GixBackend
        .repository_watch_info(&bare)
        .unwrap()
        .unwrap();
    assert!(!before.worktrees.contains(&bare));
    let linked_info = crate::GixBackend
        .repository_watch_info(&linked)
        .unwrap()
        .unwrap();
    assert!(linked_info.worktrees.contains(&linked));
    let repo = open_repo(&bare);
    let preview = repo.preview_disable_bare_mode().unwrap();
    repo.disable_bare_mode(&preview).unwrap();
    let after = crate::GixBackend
        .repository_watch_info(&bare)
        .unwrap()
        .unwrap();
    assert!(after.worktrees.contains(&bare));
    assert!(after.git_dirs.contains(&bare.join(".git")));
    assert!(!after.git_dirs.contains(&bare));
}

#[test]
fn externally_changed_mode_invalidates_the_old_native_topology() {
    let directory = tempfile::tempdir().unwrap();
    init_test_repo(directory.path());
    let retained = crate::GixBackend.open(directory.path()).unwrap();
    assert_eq!(
        retained.repository_mode().unwrap(),
        RepositoryMode::WorkingTree
    );
    git_success(directory.path(), &["config", "core.bare", "true"]);
    assert!(matches!(
        retained.repository_mode().unwrap_err().kind(),
        ErrorKind::RepositoryChanged
    ));
    let reopened = crate::GixBackend.open(directory.path()).unwrap();
    assert_eq!(reopened.repository_mode().unwrap(), RepositoryMode::Bare);
    assert!(!std::sync::Arc::ptr_eq(&retained, &reopened));
}

#[test]
fn included_bare_config_does_not_override_native_repository_mode() {
    let directory = tempfile::tempdir().unwrap();
    init_test_repo(directory.path());
    git_success(
        directory.path(),
        &["config", "include.path", "later-config"],
    );
    let retained = crate::GixBackend.open(directory.path()).unwrap();
    fs::write(
        directory.path().join(".git/later-config"),
        "[core]\n\tbare = true\n",
    )
    .unwrap();
    assert_eq!(
        git_stdout(
            directory.path(),
            &["config", "--includes", "--get", "core.bare"]
        ),
        "true"
    );
    assert_eq!(
        retained.repository_mode().unwrap(),
        RepositoryMode::WorkingTree
    );
    let reopened = crate::GixBackend.open(directory.path()).unwrap();
    assert_eq!(
        reopened.repository_mode().unwrap(),
        RepositoryMode::WorkingTree
    );
}

#[test]
fn a_bare_folder_named_dot_git_keeps_its_coordination_identity_after_conversion() {
    let (directory, bare) = bare_fixture();
    let root = directory.path().join(".git");
    fs::rename(bare, &root).unwrap();
    assert_eq!(
        crate::repo::bare_lifecycle::coordination_path(&root).unwrap(),
        crate::repo::bare_lifecycle::coordination_path(&root.join(".git")).unwrap()
    );
    let repo = crate::GixBackend.open(&root).unwrap();
    assert_eq!(repo.repository_mode().unwrap(), RepositoryMode::Bare);
    let preview = repo.preview_disable_bare_mode().unwrap();
    repo.disable_bare_mode(&preview).unwrap();
    assert_eq!(
        crate::GixBackend
            .open(&root)
            .unwrap()
            .repository_mode()
            .unwrap(),
        RepositoryMode::WorkingTree
    );
    assert_eq!(
        fs::read_to_string(root.join("tracked.txt")).unwrap(),
        "original\n"
    );
    assert!(matches!(
        repo.repository_mode().unwrap_err().kind(),
        ErrorKind::RepositoryChanged
    ));
}
