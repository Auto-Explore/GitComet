use crate::formats;
use formats::{fixture, git};
use gitcomet_core::domain::{BareConversionHead, CommitId, DiffArea, DiffTarget, HistoryMode};
use gitcomet_core::services::{CancellationToken, GitBackend, SequencerState};
use gitcomet_git_gix::GixBackend;
use std::{fs, path::Path};

#[test]
fn reftable_open_stage_commit_and_uncommitted_blame() {
    if !formats::reftable_available() {
        return;
    }
    for hash in ["sha1", "sha256"] {
        formats::open_stage_commit_and_uncommitted_blame(hash, "reftable");
    }
}

#[test]
fn reftable_history_loose_commit_graph_and_packs() {
    if !formats::reftable_available() {
        return;
    }
    for hash in ["sha1", "sha256"] {
        formats::history_loose_commit_graph_and_packs(hash, "reftable");
    }
}

#[test]
fn reftable_status_diff_commit_reflog_and_revision_lookups() {
    if !formats::reftable_available() {
        return;
    }
    for hash in ["sha1", "sha256"] {
        formats::status_diff_commit_reflog_and_file_history(hash, "reftable");
        formats::revision_lookups(hash, "reftable");
    }
}

fn commit_file(path: &Path, text: &str, message: &str) -> CommitId {
    fs::write(path.join("file.txt"), text).unwrap();
    git(path, &["add", "."]);
    git(path, &["commit", "-m", message]);
    CommitId(git(path, &["rev-parse", "HEAD"]).into())
}

#[test]
fn reftable_focus_reads_local_branches_remote_branches_and_remotes() {
    use gitcomet_core::domain::{HistoryFocus, HistoryFocusSet};
    use gitcomet_core::services::{HistoryReadRequest, HistoryReadResult};

    if !formats::reftable_available() {
        return;
    }
    for hash in ["sha1", "sha256"] {
        let dir = fixture(hash, "reftable");
        let root = commit_file(dir.path(), "root\n", "root");
        git(dir.path(), &["branch", "side"]);
        // A configured remote: focus resolves remote targets through its
        // fetch refspecs, as the sidebar lists them.
        git(
            dir.path(),
            &["remote", "add", "fork", "https://example.invalid/fork.git"],
        );
        git(
            dir.path(),
            &["update-ref", "refs/remotes/fork/side", root.as_ref()],
        );
        commit_file(dir.path(), "root\nchild\n", "child");
        let repo = GixBackend.open(dir.path()).unwrap();

        for target in [
            HistoryFocus::local_branch("side"),
            HistoryFocus::remote_branch("fork", "side"),
            HistoryFocus::remote("fork"),
        ] {
            let result = repo
                .read_history(
                    &HistoryMode::FirstParent.into(),
                    None,
                    &HistoryFocusSet::from_iter([target]),
                    &HistoryReadRequest::Page {
                        limit: 100,
                        cursor: None,
                        snapshot: None,
                    },
                    &CancellationToken::new(),
                    &mut |_| {},
                )
                .unwrap();
            let HistoryReadResult::Page { page, .. } = result else {
                panic!("expected a focused history page");
            };
            assert_eq!(page.commits.len(), 1);
            assert_eq!(page.commits[0].id, root);
        }
    }
}

#[test]
fn bare_conversion_uses_native_reftable_head_instead_of_the_placeholder() {
    if !formats::reftable_available() {
        return;
    }
    for hash in ["sha1", "sha256"] {
        let source = fixture(hash, "reftable");
        let commit = commit_file(source.path(), "root\n", "root");
        for head in [
            BareConversionHead::Branch {
                name: "main".into(),
                commit: Some(commit.clone()),
            },
            BareConversionHead::Detached {
                commit: commit.clone(),
            },
            BareConversionHead::Branch {
                name: "unborn".into(),
                commit: None,
            },
        ] {
            let directory = tempfile::tempdir().unwrap();
            let bare = directory.path().join("history.git");
            git(
                directory.path(),
                &[
                    "clone",
                    "--bare",
                    "--ref-format=reftable",
                    source.path().to_str().unwrap(),
                    bare.to_str().unwrap(),
                ],
            );
            match &head {
                BareConversionHead::Branch { name, .. } => {
                    git(
                        &bare,
                        &["symbolic-ref", "HEAD", &format!("refs/heads/{name}")],
                    );
                }
                BareConversionHead::Detached { commit } => {
                    git(
                        &bare,
                        &["update-ref", "--no-deref", "HEAD", commit.as_ref()],
                    );
                }
            }
            assert_eq!(
                fs::read(bare.join("HEAD")).unwrap(),
                b"ref: refs/heads/.invalid\n"
            );
            let repo = GixBackend.open(&bare).unwrap();
            let preview = repo.preview_disable_bare_mode().unwrap();
            assert_eq!(preview.head, head);
            repo.disable_bare_mode(&preview).unwrap();
            assert_eq!(git(&bare, &["rev-parse", "--show-ref-format"]), "reftable");
            assert_eq!(git(&bare, &["rev-parse", "--is-bare-repository"]), "false");
            match head {
                BareConversionHead::Branch { commit: None, name } => {
                    assert_eq!(git(&bare, &["symbolic-ref", "--short", "HEAD"]), name);
                    assert!(!bare.join("file.txt").exists());
                }
                BareConversionHead::Branch {
                    commit: Some(commit),
                    ..
                }
                | BareConversionHead::Detached { commit } => {
                    assert_eq!(git(&bare, &["rev-parse", "HEAD"]), commit.as_ref());
                    assert_eq!(fs::read(bare.join("file.txt")).unwrap(), b"root\n");
                }
            }
        }
    }
}

#[test]
fn files_and_reftable_twins_have_the_same_refs_status_history_and_reflogs() {
    if !formats::reftable_available() {
        return;
    }
    for hash in ["sha1", "sha256"] {
        let mut snapshots = Vec::new();
        for refs in ["files", "reftable"] {
            let dir = fixture(hash, refs);
            let root = commit_file(dir.path(), "root\n", "root");
            git(dir.path(), &["branch", "side"]);
            let head = commit_file(dir.path(), "root\nchild\n", "child");
            git(dir.path(), &["tag", "-a", "inner", "-m", "inner"]);
            git(dir.path(), &["tag", "-a", "outer", "inner", "-m", "outer"]);
            git(
                dir.path(),
                &["update-ref", "refs/remotes/origin/main", head.as_ref()],
            );
            git(
                dir.path(),
                &[
                    "symbolic-ref",
                    "refs/remotes/origin/HEAD",
                    "refs/remotes/origin/main",
                ],
            );
            git(
                dir.path(),
                &["symbolic-ref", "refs/heads/alias", "refs/heads/side"],
            );
            let repo = GixBackend.open(dir.path()).unwrap();
            let tags = repo
                .list_tags(&gitcomet_core::services::CancellationToken::new())
                .unwrap();
            assert_eq!(tags[0].target, head);
            assert_eq!(tags[1].target, head);
            let branches = repo
                .list_branches(&gitcomet_core::services::CancellationToken::new())
                .unwrap();
            assert!(
                branches
                    .iter()
                    .any(|b| b.name == "alias" && b.target == root)
            );
            assert!(!branches.iter().any(|b| b.name == "dangling"));
            let logs = repo.reflog_head(20).unwrap();
            assert_eq!(logs.len(), 2);
            assert_eq!(
                repo.commit_details(&CommitId("main~1".into()))
                    .unwrap()
                    .message
                    .trim(),
                "root"
            );
            assert_eq!(
                repo.commit_details(&CommitId(head.as_ref()[..12].into()))
                    .unwrap()
                    .message
                    .trim(),
                "child"
            );
            git(dir.path(), &["mv", "file.txt", "renamed.txt"]);
            fs::write(dir.path().join("renamed.txt"), "root\nchild\nlocal\n").unwrap();
            fs::create_dir(dir.path().join("untracked")).unwrap();
            fs::write(dir.path().join("untracked/new.txt"), "new\n").unwrap();
            let status = repo
                .status(&gitcomet_core::services::CancellationToken::new())
                .unwrap();
            assert_eq!(status.staged.len(), 1);
            assert!(
                repo.diff_file_text(
                    &DiffTarget::working_tree("renamed.txt".into(), DiffArea::Staged),
                    None,
                    &gitcomet_core::services::CancellationToken::new()
                )
                .unwrap()
                .is_some()
            );
            let remote_branches = repo
                .list_remote_branches(&gitcomet_core::services::CancellationToken::new())
                .unwrap();
            let snapshot = format!("{branches:?} {tags:?} {remote_branches:?} {logs:?} {status:?}");
            snapshots.push(snapshot);
            // Both storages skip dangling symrefs, such as the `origin/HEAD`
            // a pruning fetch leaves behind, as Git's for-each-ref does.
            git(
                dir.path(),
                &["symbolic-ref", "refs/heads/dangling", "refs/heads/missing"],
            );
            git(
                dir.path(),
                &[
                    "symbolic-ref",
                    "refs/remotes/fork/HEAD",
                    "refs/remotes/fork/main",
                ],
            );
            assert!(
                !repo
                    .list_branches(&gitcomet_core::services::CancellationToken::new())
                    .unwrap()
                    .iter()
                    .any(|b| b.name == "dangling")
            );
            assert_eq!(
                repo.list_remote_branches(&gitcomet_core::services::CancellationToken::new())
                    .unwrap(),
                remote_branches
            );
            git(dir.path(), &["reset", "--hard"]);
            // Mutating refs outside the backend invalidates the reader and metadata cache.
            git(
                dir.path(),
                &["update-ref", "refs/heads/side", head.as_ref()],
            );
            assert!(
                repo.list_branches(&gitcomet_core::services::CancellationToken::new())
                    .unwrap()
                    .iter()
                    .any(|b| b.name == "side" && b.target == head)
            );
            assert_eq!(
                repo.build_history_index(
                    &HistoryMode::AllBranches.into(),
                    None,
                    &Default::default(),
                    &CancellationToken::new(),
                    &mut |_| {}
                )
                .unwrap()
                .unwrap()
                .len(),
                2
            );
        }
        assert_eq!(snapshots[0], snapshots[1], "{hash} storage twins");
    }
}

#[test]
fn reftable_branch_writes_work_without_identity_and_respect_linked_worktrees() {
    if !formats::reftable_available() {
        return;
    }
    for hash in ["sha1", "sha256"] {
        let dir = fixture(hash, "reftable");
        let root = commit_file(dir.path(), "root\n", "root");
        let repo = GixBackend.open(dir.path()).unwrap();
        git(dir.path(), &["config", "--unset", "user.email"]);
        git(dir.path(), &["config", "--unset", "user.name"]);
        repo.create_branch("topic", &root).unwrap();
        assert!(repo.create_branch("topic", &root).is_err());
        repo.rename_branch("topic", "renamed").unwrap();
        repo.delete_branch_force("renamed").unwrap();
        assert!(
            !repo
                .list_branches(&gitcomet_core::services::CancellationToken::new())
                .unwrap()
                .iter()
                .any(|b| b.name == "renamed")
        );
        repo.create_branch("linked", &root).unwrap();
        let linked_parent = tempfile::tempdir().unwrap();
        let linked = linked_parent.path().join("linked");
        git(
            dir.path(),
            &["worktree", "add", linked.to_str().unwrap(), "linked"],
        );
        let linked_repo = GixBackend.open(&linked).unwrap();
        assert_eq!(
            linked_repo
                .current_branch(&gitcomet_core::services::CancellationToken::new())
                .unwrap(),
            "linked"
        );
        assert_eq!(
            linked_repo
                .log_head_page(10, None, &gitcomet_core::services::CancellationToken::new())
                .unwrap()
                .commits[0]
                .id,
            root
        );
        assert!(repo.delete_branch_force("linked").is_err());
        linked_repo.checkout_commit(&root).unwrap();
        assert_eq!(
            linked_repo
                .current_branch(&gitcomet_core::services::CancellationToken::new())
                .unwrap(),
            "HEAD"
        );
        assert_eq!(
            repo.current_branch(&gitcomet_core::services::CancellationToken::new())
                .unwrap(),
            "main"
        );
        repo.delete_branch_force("linked").unwrap();
        repo.create_tag_with_output("light", root.as_ref(), None, false)
            .unwrap();
        assert_eq!(
            repo.list_tags(&gitcomet_core::services::CancellationToken::new())
                .unwrap()[0]
                .target,
            root
        );
        repo.delete_tag_with_output("light").unwrap();
        assert!(
            repo.list_tags(&gitcomet_core::services::CancellationToken::new())
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn resolved_single_cherry_pick_survives_unstage_all() {
    if !formats::reftable_available() {
        return;
    }
    for hash in ["sha1", "sha256"] {
        for refs in ["files", "reftable"] {
            let dir = fixture(hash, refs);
            commit_file(dir.path(), "base\n", "base");
            git(dir.path(), &["checkout", "-b", "side"]);
            let source = commit_file(dir.path(), "side\n", "side");
            git(dir.path(), &["checkout", "main"]);
            commit_file(dir.path(), "main\n", "main");
            let repo = GixBackend.open(dir.path()).unwrap();
            assert!(repo.cherry_pick(&source).is_err());
            assert_eq!(
                repo.sequencer_state(&gitcomet_core::services::CancellationToken::new())
                    .unwrap(),
                SequencerState::CherryPick
            );
            fs::write(dir.path().join("file.txt"), "resolved\n").unwrap();
            repo.stage(&[Path::new("file.txt")]).unwrap();
            repo.unstage(&[]).unwrap();
            assert_eq!(
                git(dir.path(), &["rev-parse", "--verify", "CHERRY_PICK_HEAD"]),
                source.as_ref()
            );
            assert_eq!(
                repo.sequencer_state(&gitcomet_core::services::CancellationToken::new())
                    .unwrap(),
                SequencerState::CherryPick
            );
            assert!(
                !repo
                    .status(&gitcomet_core::services::CancellationToken::new())
                    .unwrap()
                    .unstaged
                    .is_empty()
            );
            repo.rebase_abort_with_output().unwrap();
            assert_eq!(
                repo.sequencer_state(&gitcomet_core::services::CancellationToken::new())
                    .unwrap(),
                SequencerState::None
            );
        }
    }
}

#[test]
fn shallow_and_no_checkout_clones_use_native_refs_and_index_fallback() {
    if !formats::reftable_available() {
        return;
    }
    for hash in ["sha1", "sha256"] {
        let source = fixture(hash, "reftable");
        commit_file(source.path(), "root\n", "root");
        let head = commit_file(source.path(), "root\nchild\n", "child");
        let parent = tempfile::tempdir().unwrap();
        let url = format!("file://{}", source.path().display());
        git(
            parent.path(),
            &[
                "clone",
                "--depth",
                "1",
                "--ref-format=reftable",
                &url,
                "shallow",
            ],
        );
        let shallow = GixBackend.open(&parent.path().join("shallow")).unwrap();
        assert_eq!(
            shallow
                .log_head_page(10, None, &gitcomet_core::services::CancellationToken::new())
                .unwrap()
                .commits
                .len(),
            1
        );
        git(
            parent.path(),
            &[
                "clone",
                "--no-checkout",
                "--ref-format=reftable",
                &url,
                "empty-index",
            ],
        );
        let cloned_path = parent.path().join("empty-index");
        assert!(!cloned_path.join(".git/index").exists());
        let cloned = GixBackend.open(&cloned_path).unwrap();
        assert_eq!(cloned.commit_details(&head).unwrap().id, head);
        assert!(
            cloned
                .diff_file_text(
                    &DiffTarget::commit(head.clone(), "file.txt".into()),
                    None,
                    &gitcomet_core::services::CancellationToken::new()
                )
                .unwrap()
                .is_some()
        );
        cloned
            .status(&gitcomet_core::services::CancellationToken::new())
            .unwrap();
        assert!(
            !cloned_path.join(".git/index").exists(),
            "reads must not create a real index"
        );
    }
}

#[test]
fn reftable_submodules() {
    if !formats::reftable_available() {
        return;
    }
    for hash in ["sha1", "sha256"] {
        formats::submodules_and_unconfigured_gitlinks(hash, "reftable");
    }
}
