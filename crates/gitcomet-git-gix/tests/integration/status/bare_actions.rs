use super::*;
use gitcomet_core::domain::{ApplyChangeTarget, ApplyFileChangeRetry, RepositoryMode};
use gitcomet_core::error::BareConversionRefusal;
use gitcomet_core::large_files::{AnnexAdjustMode, LargeFileCommand};
use gitcomet_core::patch_action::{PatchOperation, SourceLineSelection};
use gitcomet_core::remote_url::RemoteUrlPolicy;
use gitcomet_core::services::{
    ComparisonOptions, GitRepository, PullMode, RebaseRange, ResetMode, Result, WorktreeCheckout,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

fn fixture() -> (tempfile::TempDir, Arc<dyn GitRepository>) {
    let dir = tempfile::tempdir().unwrap();
    init_bare(dir.path(), true);
    run_git(dir.path(), &["config", "user.name", "Test"]);
    run_git(dir.path(), &["config", "user.email", "test@example.com"]);
    run_git(dir.path(), &["config", "commit.gpgsign", "false"]);
    let repo = GixBackend.open(dir.path()).unwrap();
    (dir, repo)
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, path: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                visit(root, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().to_owned(),
                    fs::read(&path).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}

fn refused<T>(root: &Path, before: &BTreeMap<PathBuf, Vec<u8>>, result: Result<T>) {
    let Err(error) = result else {
        panic!("bare operation unexpectedly succeeded")
    };
    assert!(
        matches!(error.kind(), ErrorKind::BareUnsupported),
        "{error:?}"
    );
    assert_eq!(&snapshot(root), before);
}

#[test]
fn direct_worktree_mutations_and_reads_refuse_bare_without_changing_git_data() {
    let (dir, repo) = fixture();
    let root = dir.path();
    let cancel = CancellationToken::new();
    let head = repo.head_commit_id().unwrap().unwrap();
    let target = DiffTarget::working_tree("config".into(), DiffArea::Unstaged);
    let selection = SourceLineSelection::default();
    let apply = ApplyChangeTarget::commit(head.clone(), "file.txt".into());
    let retry = ApplyFileChangeRetry {
        target: apply.clone(),
        head: Some(head.clone()),
        index: Vec::new(),
    };
    let range = RebaseRange {
        onto: head.clone(),
        upstream: head.clone(),
        expected_head: head.clone(),
        preserve_merges: false,
    };
    let before = snapshot(root);
    macro_rules! reject {
        ($call:expr) => {
            refused(root, &before, $call)
        };
    }
    reject!(repo.checkout_branch("main"));
    reject!(repo.create_branch_force_and_checkout("created", &head));
    reject!(repo.checkout_remote_branch(
        "origin",
        "main",
        "created",
        CheckoutRemoteBranchMode::Create
    ));
    reject!(repo.checkout_commit(&head));
    reject!(repo.stage(&[Path::new("config")]));
    reject!(repo.unstage(&[]));
    reject!(repo.commit("blocked"));
    reject!(repo.commit_with_outcome("blocked"));
    reject!(repo.commit_with_identity("blocked", None));
    reject!(repo.commit_amend("blocked"));
    reject!(repo.commit_amend_with_output("blocked"));
    reject!(repo.commit_amend_with_outcome("blocked"));
    reject!(repo.commit_amend_with_identity("blocked", None));
    reject!(repo.discard_worktree_changes(&[Path::new("config")]));
    reject!(repo.pull(PullMode::Default, false));
    reject!(repo.pull_branch_with_output("origin", "main", false));
    reject!(repo.merge_ref_with_output("HEAD"));
    reject!(repo.squash_ref_with_output("HEAD"));
    reject!(repo.merge_abort_with_output());
    reject!(repo.rebase_with_output("HEAD"));
    reject!(repo.rebase_range_with_output(&range));
    reject!(repo.rebase_continue_with_output());
    reject!(repo.rebase_abort_with_output());
    reject!(repo.interactive_rebase_with_output("HEAD", &[]));
    reject!(repo.interactive_cherry_pick_with_output(&[], false));
    reject!(repo.cherry_pick(&head));
    reject!(repo.cherry_pick_with_output(&head, true, None));
    reject!(repo.revert_with_output(&head, true, None));
    reject!(repo.apply_file_change_with_output(&apply, true));
    reject!(repo.commit_applied_file_change_with_output(&retry));
    reject!(repo.reset_with_output("HEAD", ResetMode::Mixed));
    reject!(repo.reset_with_output("HEAD", ResetMode::Hard));
    reject!(repo.stash_create("blocked", true));
    reject!(repo.stash_apply(0));
    reject!(repo.stash_drop(0));
    reject!(repo.apply_patch_with_output(Path::new("missing.patch")));
    reject!(repo.apply_unified_patch_to_index_with_output(b"", false));
    reject!(repo.apply_unified_patch_to_worktree_with_output(b"", false));
    reject!(repo.diff_action_snapshot(&target, &cancel));
    reject!(repo.prepare_diff_patch(&target, PatchOperation::Stage, &selection, "", "", &cancel));
    reject!(repo.conflict_file_stages(Path::new("config")));
    reject!(repo.conflict_session(Path::new("config"), None));
    reject!(repo.checkout_conflict_side(Path::new("config"), ConflictSide::Ours));
    reject!(repo.checkout_conflict_base(Path::new("config")));
    reject!(repo.accept_conflict_deletion(Path::new("config")));
    reject!(repo.launch_mergetool(Path::new("config")));
    reject!(repo.list_worktree_files());
    reject!(repo.diff_unified(&target));
    reject!(repo.diff_parsed(&target, None, &cancel));
    reject!(repo.diff_file_text(&target, None, &cancel));
    reject!(repo.diff_file_image(&target, &cancel));
    reject!(repo.diff_preview_text_file(&target, DiffPreviewTextSide::New, &cancel));
    reject!(repo.submodule_diff_summary(&target, &cancel));
    reject!(repo.compare_files(&head, None, &ComparisonOptions::default(), &cancel));
    reject!(repo.blame_worktree_file(Path::new("config"), DiffArea::Unstaged));
    reject!(repo.uncommitted_line_stats(&cancel));
    reject!(repo.uncommitted_line_stats_for_status(&RepoStatus::default(), &cancel));
    reject!(repo.uncommitted_large_files_for_status(&RepoStatus::default(), &cancel));
    reject!(repo.check_submodule_add_trust("source", Path::new("sub"), RemoteUrlPolicy::default()));
    reject!(repo.check_submodule_update_trust(RemoteUrlPolicy::default()));
    reject!(repo.check_submodule_load_trust(Path::new("sub"), RemoteUrlPolicy::default()));
    reject!(repo.add_submodule_with_output(
        "source",
        Path::new("sub"),
        None,
        None,
        false,
        &[],
        RemoteUrlPolicy::default()
    ));
    reject!(repo.update_submodules_with_output(&[], RemoteUrlPolicy::default()));
    reject!(repo.load_submodule_with_output(Path::new("sub"), &[], RemoteUrlPolicy::default()));
    reject!(repo.change_submodule_pointer_with_output(Path::new("sub"), "HEAD"));
    reject!(repo.remove_submodule_with_output(Path::new("sub")));
    let worktree_range = DiffTarget::commit_range(head, None, None);
    reject!(repo.diff_unified(&worktree_range));
}

#[test]
fn bare_history_refs_and_committed_files_remain_usable() {
    let (dir, repo) = fixture();
    let cancel = CancellationToken::new();
    let head = repo.head_commit_id().unwrap().unwrap();
    let parent = CommitId(run_git_output(dir.path(), &["rev-parse", "HEAD^"]).into());
    assert_eq!(repo.repository_mode().unwrap(), RepositoryMode::Bare);
    assert_eq!(repo.list_tree_files_at_commit(&head).unwrap().len(), 1);
    assert_eq!(
        repo.blame_file(Path::new("file.txt"), None).unwrap().len(),
        1
    );
    let target = DiffTarget::commit(head.clone(), "file.txt".into());
    assert!(repo.diff_unified(&target).unwrap().contains("+two"));
    assert!(repo.diff_parsed(&target, None, &cancel).is_ok());
    assert!(
        repo.diff_file_text(&target, None, &cancel)
            .unwrap()
            .is_some()
    );
    assert!(
        repo.diff_preview_text_file(&target, DiffPreviewTextSide::New, &cancel)
            .unwrap()
            .is_some()
    );
    assert!(
        repo.compare_files(&parent, Some(&head), &ComparisonOptions::default(), &cancel)
            .is_ok()
    );
    assert!(repo.stash_list(&cancel).unwrap().is_empty());
    assert!(repo.list_submodules(&cancel).unwrap().is_empty());
    repo.create_branch("topic", &head).unwrap();
    repo.rename_branch("topic", "renamed").unwrap();
    repo.delete_branch_force("renamed").unwrap();
    repo.create_tag_with_output("bare-tag", head.as_ref(), None, false)
        .unwrap();
    assert_eq!(repo.list_tags(&cancel).unwrap().len(), 1);
    repo.delete_tag_with_output("bare-tag").unwrap();
    let export = tempfile::tempdir().unwrap();
    repo.export_patch_with_output(&head, &export.path().join("change.patch"))
        .unwrap();
    assert!(!dir.path().join("index").exists());
    assert!(!dir.path().join("file.txt").exists());
}

#[test]
fn bare_stash_viewing_preserves_a_real_stash_when_drop_is_refused() {
    let (dir, bare) = fixture();
    let linked_dir = tempfile::tempdir().unwrap();
    let linked = linked_dir.path().join("checkout");
    bare.add_worktree_with_output(
        &linked,
        &WorktreeCheckout::NewBranch {
            name: "linked".into(),
            start: None,
        },
    )
    .unwrap();
    write(&linked, "file.txt", "stashed change\n");
    let checkout = GixBackend.open(&linked).unwrap();
    checkout.stash_create("preserve me", false).unwrap();
    let stashes = bare.stash_list(&CancellationToken::new()).unwrap();
    assert_eq!(stashes.len(), 1);
    let before = snapshot(dir.path());
    refused(dir.path(), &before, bare.stash_drop(0));
    assert_eq!(bare.stash_list(&CancellationToken::new()).unwrap(), stashes);
}

#[test]
fn linked_worktree_with_shared_bare_config_accepts_worktree_actions_and_patches() {
    let (dir, bare) = fixture();
    let linked_dir = tempfile::tempdir().unwrap();
    let linked = linked_dir.path().join("checkout");
    bare.add_worktree_with_output(
        &linked,
        &WorktreeCheckout::NewBranch {
            name: "linked".into(),
            start: None,
        },
    )
    .unwrap();
    let repo = GixBackend.open(&linked).unwrap();
    assert_eq!(run_git_output(dir.path(), &["config", "core.bare"]), "true");
    assert_eq!(repo.repository_mode().unwrap(), RepositoryMode::WorkingTree);
    write(&linked, "file.txt", "three\n");
    let target = DiffTarget::working_tree("file.txt".into(), DiffArea::Unstaged);
    let prepared = repo
        .prepare_diff_patch(
            &target,
            PatchOperation::Stage,
            &SourceLineSelection {
                old: BTreeSet::from([1]),
                new: BTreeSet::from([1]),
            },
            "two\n",
            "three\n",
            &CancellationToken::new(),
        )
        .unwrap();
    let before = snapshot(dir.path());
    refused(
        dir.path(),
        &before,
        bare.apply_prepared_diff_patch(&prepared),
    );
    repo.apply_prepared_diff_patch(&prepared).unwrap();
    repo.commit("linked change").unwrap();
    assert_eq!(run_git_output(&linked, &["show", "HEAD:file.txt"]), "three");
    assert_eq!(
        run_git_output(dir.path(), &["show", "HEAD:file.txt"]),
        "two"
    );
}

#[test]
fn bare_head_rewrites_refuse_occupied_branches_even_for_missing_linked_paths() {
    let (dir, repo) = fixture();
    let linked_dir = tempfile::tempdir().unwrap();
    let linked = linked_dir.path().join("checkout");
    repo.add_worktree_with_output(
        &linked,
        &WorktreeCheckout::Existing {
            reference: "main".into(),
        },
    )
    .unwrap();
    let head = repo.head_commit_id().unwrap().unwrap();
    let oldest = CommitId(run_git_output(dir.path(), &["rev-parse", "HEAD^"]).into());
    for missing in [false, true] {
        if missing {
            fs::remove_dir_all(&linked).unwrap();
        }
        let before = snapshot(dir.path());
        for result in [
            repo.reset_with_output(oldest.as_ref(), ResetMode::Soft),
            repo.squash_commits_with_output(&oldest, &head, "blocked"),
        ] {
            let error = result.unwrap_err();
            assert!(
                matches!(
                    error.kind(),
                    ErrorKind::BareConversionRefused(BareConversionRefusal::BranchInUse { .. })
                ),
                "{error:?}"
            );
            assert_eq!(snapshot(dir.path()), before);
        }
    }
}

#[test]
fn bare_soft_reset_allows_unoccupied_and_detached_head() {
    for detached in [false, true] {
        let (dir, repo) = fixture();
        let head = repo.head_commit_id().unwrap().unwrap();
        let parent = run_git_output(dir.path(), &["rev-parse", "HEAD^"]);
        if detached {
            run_git(
                dir.path(),
                &["update-ref", "--no-deref", "HEAD", head.as_ref()],
            );
        }
        repo.reset_with_output(&parent, ResetMode::Soft).unwrap();
        assert_eq!(repo.head_commit_id().unwrap().unwrap().as_ref(), parent);
        assert!(!dir.path().join("index").exists());
        assert!(!dir.path().join("file.txt").exists());
    }
}

#[test]
fn bare_object_squash_preserves_tree_and_allows_detached_head_with_occupied_main() {
    for detached in [false, true] {
        let (dir, repo) = fixture();
        let oldest = repo.head_commit_id().unwrap().unwrap();
        let third = run_git_output(
            dir.path(),
            &["commit-tree", "HEAD^{tree}", "-p", "HEAD", "-m", "third"],
        );
        run_git(dir.path(), &["update-ref", "HEAD", &third]);
        let linked_dir = tempfile::tempdir().unwrap();
        if detached {
            repo.add_worktree_with_output(
                &linked_dir.path().join("checkout"),
                &WorktreeCheckout::Existing {
                    reference: "main".into(),
                },
            )
            .unwrap();
            run_git(dir.path(), &["update-ref", "--no-deref", "HEAD", &third]);
        }
        let tree = run_git_output(dir.path(), &["rev-parse", "HEAD^{tree}"]);
        repo.squash_commits_with_output(&oldest, &CommitId(third.clone().into()), "combined")
            .unwrap();
        assert_eq!(
            run_git_output(dir.path(), &["rev-parse", "HEAD^{tree}"]),
            tree
        );
        assert_eq!(
            run_git_output(dir.path(), &["log", "-1", "--format=%s"]),
            "combined"
        );
        assert_eq!(
            run_git_output(dir.path(), &["rev-list", "--count", "HEAD"]),
            "2"
        );
        if detached {
            assert_eq!(run_git_output(dir.path(), &["rev-parse", "main"]), third);
        }
        assert!(!dir.path().join("index").exists());
        assert!(!dir.path().join("file.txt").exists());
    }
}

#[test]
fn bare_fetch_and_push_use_local_remotes_without_a_checkout() {
    let (dir, repo) = fixture();
    let remote = tempfile::tempdir().unwrap();
    init_bare(remote.path(), false);
    repo.add_remote_with_output(
        "origin",
        &git_path_arg(remote.path()),
        RemoteUrlPolicy::default(),
    )
    .unwrap();
    repo.push_set_upstream("origin", "main").unwrap();
    assert_eq!(
        run_git_output(remote.path(), &["rev-parse", "main"]),
        repo.head_commit_id().unwrap().unwrap().as_ref()
    );
    let next = run_git_output(
        dir.path(),
        &[
            "commit-tree",
            "HEAD^{tree}",
            "-p",
            "HEAD",
            "-m",
            "remote change",
        ],
    );
    run_git(remote.path(), &["fetch", &git_path_arg(dir.path()), &next]);
    run_git(
        remote.path(),
        &["update-ref", "refs/heads/remote-topic", &next],
    );
    repo.fetch_all(false).unwrap();
    assert_eq!(
        run_git_output(
            dir.path(),
            &["rev-parse", "refs/remotes/origin/remote-topic"]
        ),
        next
    );
    repo.push().unwrap();
    assert!(!dir.path().join("index").exists());
}

#[test]
fn direct_large_file_worktree_commands_refuse_before_effects_or_annex_restage() {
    let (dir, repo) = fixture();
    let paths = vec![PathBuf::from("config")];
    let before = snapshot(dir.path());
    for command in [
        LargeFileCommand::LfsPull { paths: Vec::new() },
        LargeFileCommand::LfsLock {
            paths: paths.clone(),
        },
        LargeFileCommand::LfsUnlock {
            paths: paths.clone(),
            force: false,
        },
        LargeFileCommand::LfsFetchForDiff {
            target: DiffTarget::working_tree("config".into(), DiffArea::Staged),
        },
        LargeFileCommand::LfsTrack {
            patterns: vec!["*.bin".into()],
            filename: false,
            lockable: false,
            renormalize: Vec::new(),
        },
        LargeFileCommand::AnnexGet {
            paths: paths.clone(),
            from: None,
        },
        LargeFileCommand::AnnexDrop {
            paths: paths.clone(),
            from: None,
            force: true,
        },
        LargeFileCommand::AnnexCopy {
            paths: paths.clone(),
            to: "origin".into(),
        },
        LargeFileCommand::AnnexMove {
            paths: paths.clone(),
            to: "origin".into(),
        },
        LargeFileCommand::AnnexAdd {
            paths: paths.clone(),
        },
        LargeFileCommand::AnnexLock {
            paths: paths.clone(),
        },
        LargeFileCommand::AnnexUnlock { paths },
        LargeFileCommand::AnnexAdjust {
            mode: AnnexAdjustMode::Unlock,
        },
        LargeFileCommand::AnnexLeaveAdjusted {
            base: "main".into(),
        },
        LargeFileCommand::AnnexWebapp,
        LargeFileCommand::AnnexStopAssistant,
    ] {
        refused(dir.path(), &before, repo.run_large_file_command(&command));
    }
}

#[test]
fn old_direct_handles_refuse_reads_and_writes_after_conversion() {
    let (dir, repo) = fixture();
    let head = repo.head_commit_id().unwrap().unwrap();
    let preview = repo.preview_disable_bare_mode().unwrap();
    repo.disable_bare_mode(&preview).unwrap();
    let before = snapshot(dir.path());
    let target = DiffTarget::commit(head.clone(), "file.txt".into());
    macro_rules! stale {
        ($call:expr) => {{
            let error = $call.err().expect("stale handle must refuse");
            assert!(
                matches!(error.kind(), ErrorKind::RepositoryChanged),
                "{error:?}"
            );
            assert_eq!(snapshot(dir.path()), before);
        }};
    }
    stale!(repo.commit_details(&head));
    stale!(repo.head_commit_id());
    stale!(repo.list_tree_files_at_commit(&head));
    stale!(repo.list_worktree_files());
    stale!(repo.diff_unified(&target));
    stale!(repo.large_file_support(&CancellationToken::new()));
    stale!(repo.create_branch("stale", &head));
    stale!(repo.stage(&[Path::new("file.txt")]));
    stale!(repo.commit("stale"));
    stale!(repo.run_large_file_command(&LargeFileCommand::AnnexInit));
    let reopened = GixBackend.open(dir.path()).unwrap();
    assert_eq!(
        reopened.repository_mode().unwrap(),
        RepositoryMode::WorkingTree
    );
    assert_eq!(reopened.head_commit_id().unwrap(), Some(head));
    assert!(
        reopened
            .list_worktree_files()
            .unwrap()
            .iter()
            .any(|entry| entry.path.as_path() == Path::new("file.txt"))
    );
}
