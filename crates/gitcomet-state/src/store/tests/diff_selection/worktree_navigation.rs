use super::*;

#[test]
fn apply_worktree_patch_emits_effect() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ApplyWorktreePatch {
            repo_id: RepoId(1),
            patch: "@@ -1 +1 @@\n-old\n+new\n".to_string().into(),
            reverse: false,
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::ApplyWorktreePatch {
            repo_id: RepoId(1),
            reverse: false,
            ..
        })]
    ));
}

/// The inline foreign diff belongs to the worktree whose file list opened it.
/// Selecting a different worktree's row must retire it, or the diff pane keeps
/// showing the previous checkout's file — chip and all — beside a file list that
/// has nothing highlighted.
#[test]
fn selecting_another_worktree_retires_the_previous_worktrees_inline_diff() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let worktree_a = PathBuf::from("/tmp/wt/a");
    let worktree_b = PathBuf::from("/tmp/wt/b");
    let inline_target = gitcomet_core::domain::DiffTarget::working_tree(
        PathBuf::from("src/lib.rs"),
        gitcomet_core::domain::DiffArea::Unstaged,
    );

    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.history_state.worktree_selection = Some(worktree_a.clone());
    repo_state.diff_state.inline_submodule_diff = Some(crate::model::InlineSubmoduleDiffState {
        origin: crate::model::ForeignDiffOrigin::Worktree {
            branch: Some("side".to_string()),
            detached: false,
        },
        submodule_repo_path: worktree_a.clone(),
        parent_submodule_path: worktree_a.clone(),
        entries: vec![crate::model::InlineSubmoduleDiffEntry {
            path: PathBuf::from("src/lib.rs"),
            kind: FileStatusKind::Modified,
            target: inline_target.clone(),
            section: crate::model::InlineSubmoduleDiffSection::LiveUnstaged,
        }]
        .into(),
        selected_ix: 0,
        target: inline_target.clone(),
        rev: 1,
        diff_rev: 1,
        diff: Loadable::NotLoaded,
        diff_file_rev: 1,
        diff_file: Loadable::NotLoaded,
        diff_file_image: Loadable::NotLoaded,
    });
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));
    let diff_state_rev_before = state.repos[0].diff_state.diff_state_rev;

    // Re-selecting the same worktree changes nothing: its own diff stays open.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectWorktreeUncommitted {
            request_id: None,
            repo_id: RepoId(1),
            path: worktree_a.clone(),
        },
    );
    assert!(
        state.repos[0].diff_state.inline_submodule_diff.is_some(),
        "re-selecting the same worktree must leave its open diff alone"
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectWorktreeUncommitted {
            request_id: None,
            repo_id: RepoId(1),
            path: worktree_b.clone(),
        },
    );

    assert!(
        state.repos[0].diff_state.inline_submodule_diff.is_none(),
        "the other worktree's inline diff must not survive the switch"
    );
    assert!(
        state.repos[0].diff_state.diff_state_rev > diff_state_rev_before,
        "the diff panes have to be told to repaint"
    );
    assert_eq!(
        state.repos[0].history_state.worktree_selection.as_ref(),
        Some(&worktree_b)
    );
}

/// Retiring a worktree's inline diff must not take the commit diff behind it
/// with it. Opening an inline foreign diff never sets `diff_target` — the target
/// still names whatever commit file was selected before — and the pane renders
/// the inline diff only *in preference* to it. Clearing the target on the way
/// out therefore deletes state the inline diff never owned, and the pane goes
/// blank instead of falling back to the commit, which is exactly what closing the
/// diff by hand (`CloseInlineSubmoduleDiff`) does.
#[test]
fn retiring_a_worktrees_inline_diff_leaves_the_commit_diff_behind_it_intact() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let worktree = PathBuf::from("/tmp/wt/a");
    let commit_target = gitcomet_core::domain::DiffTarget::commit(
        CommitId("c0".into()),
        PathBuf::from("src/main.rs"),
    );
    let inline_target = gitcomet_core::domain::DiffTarget::working_tree(
        PathBuf::from("src/lib.rs"),
        gitcomet_core::domain::DiffArea::Unstaged,
    );

    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    // The commit file the user opened first, still selected underneath.
    repo_state.set_diff_target(Some(commit_target.clone()));
    repo_state.history_state.selected_commit = Some(CommitId("c0".into()));
    // Then a worktree row, and a file inside it.
    repo_state.history_state.worktree_selection = Some(worktree.clone());
    repo_state.diff_state.inline_submodule_diff = Some(crate::model::InlineSubmoduleDiffState {
        origin: crate::model::ForeignDiffOrigin::Worktree {
            branch: Some("side".to_string()),
            detached: false,
        },
        submodule_repo_path: worktree.clone(),
        parent_submodule_path: worktree.clone(),
        entries: vec![crate::model::InlineSubmoduleDiffEntry {
            path: PathBuf::from("src/lib.rs"),
            kind: FileStatusKind::Modified,
            target: inline_target.clone(),
            section: crate::model::InlineSubmoduleDiffSection::LiveUnstaged,
        }]
        .into(),
        selected_ix: 0,
        target: inline_target,
        rev: 1,
        diff_rev: 1,
        diff: Loadable::NotLoaded,
        diff_file_rev: 1,
        diff_file: Loadable::NotLoaded,
        diff_file_image: Loadable::NotLoaded,
    });
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));

    // Selecting a commit clears the worktree selection as a side effect, which is
    // what orphans the inline diff.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectCommit {
            request_id: None,
            repo_id: RepoId(1),
            commit_id: CommitId("c1".into()),
        },
    );

    assert!(
        state.repos[0].diff_state.inline_submodule_diff.is_none(),
        "the orphaned worktree diff still has to be retired"
    );
    assert_eq!(
        state.repos[0].diff_state.diff_target.as_ref(),
        Some(&commit_target),
        "the commit file selected behind it must survive so the pane can fall back to it"
    );
}

/// A worktree row exists only while that worktree is dirty. When the scan stops
/// reporting it — committed, stashed, reverted — a selection pointing at the
/// vanished row would leave the details pane rendering nothing at all, with no
/// way back short of selecting a commit.
///
/// A scan that *failed* is the opposite case and is covered separately below:
/// it reports nothing about any worktree, so it is not evidence that this one
/// went clean.
#[test]
fn a_selection_on_a_worktree_that_went_clean_is_dropped() {
    use gitcomet_core::domain::WorktreeDirtySummary;

    let dirty = |path: &str| WorktreeDirtySummary {
        path: PathBuf::from(path),
        head: Some(CommitId("tip".into())),
        branch: Some("side".to_string()),
        detached: false,
        added: 1,
        modified: 0,
        deleted: 0,
        staged: Vec::new(),
        unstaged: Vec::new(),
        line_stats: Default::default(),
    };

    {
        let (label, result) = (
            "a scan that no longer lists it",
            Ok(vec![dirty("/tmp/wt/other")]),
        );
        let mut repos: RepositoryRuntimes = Default::default();
        let id_alloc = AtomicU64::new(2);
        let mut state = AppState::test_default();
        let mut repo_state = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        );
        repo_state.set_worktree_dirty(Loadable::Ready(vec![dirty("/tmp/wt/a")]));
        repo_state.history_state.worktree_selection = Some(PathBuf::from("/tmp/wt/a"));
        state.repos.push(repo_state);
        state.active_repo = Some(RepoId(1));

        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::WorktreeDirtyLoaded {
                repo_id: RepoId(1),
                scope: crate::model::WorktreeDirtyScope::All,
                result,
            }),
        );

        assert!(
            state.repos[0].history_state.worktree_selection.is_none(),
            "{label} must drop the selection it can no longer render"
        );
    }
}

/// The counterpart: a worktree that is still dirty keeps its selection, or every
/// rescan would bounce the user out of the row they are reading.
#[test]
fn a_selection_on_a_still_dirty_worktree_survives_a_rescan() {
    use gitcomet_core::domain::WorktreeDirtySummary;

    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let selected = PathBuf::from("/tmp/wt/a");
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.history_state.worktree_selection = Some(selected.clone());
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::WorktreeDirtyLoaded {
            repo_id: RepoId(1),
            scope: crate::model::WorktreeDirtyScope::All,
            result: Ok(vec![WorktreeDirtySummary {
                path: selected.clone(),
                head: Some(CommitId("tip".into())),
                branch: None,
                detached: true,
                added: 0,
                modified: 2,
                deleted: 0,
                staged: Vec::new(),
                unstaged: Vec::new(),
                line_stats: Default::default(),
            }]),
        }),
    );

    assert_eq!(
        state.repos[0].history_state.worktree_selection.as_ref(),
        Some(&selected)
    );
}

/// A scan-level failure means the scan never ran -- a cancelled load, a repo
/// handle that is gone, git unavailable -- not that the worktrees are clean.
/// Taking it as an answer blanked every row and, through the gone-row check,
/// dropped the selection and closed whatever inline diff was open.
#[test]
fn a_failed_worktree_scan_keeps_the_rows_and_the_selection() {
    use gitcomet_core::domain::WorktreeDirtySummary;

    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let selected = PathBuf::from("/tmp/wt/a");
    let ready = vec![WorktreeDirtySummary {
        path: selected.clone(),
        head: Some(CommitId("tip".into())),
        branch: Some("side".to_string()),
        detached: false,
        added: 1,
        modified: 0,
        deleted: 0,
        staged: Vec::new(),
        unstaged: Vec::new(),
        line_stats: Default::default(),
    }];
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.set_worktree_dirty(Loadable::Ready(ready.clone()));
    repo_state.history_state.worktree_selection = Some(selected.clone());
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::WorktreeDirtyLoaded {
            repo_id: RepoId(1),
            scope: crate::model::WorktreeDirtyScope::All,
            result: Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Backend("scan failed".to_string()),
            )),
        }),
    );

    assert!(
        matches!(&state.repos[0].worktree_dirty, Loadable::Ready(dirty) if dirty.as_slice() == ready.as_slice()),
        "the last known counts must stay on screen, got {:?}",
        state.repos[0].worktree_dirty
    );
    assert_eq!(
        state.repos[0].history_state.worktree_selection.as_ref(),
        Some(&selected),
        "a scan that never ran must not deselect the row the user is reading"
    );
}

/// An open worktree diff carries the entry list its row was clicked with, while
/// the rows themselves are rebuilt from every scan. A rescan that adds or removes
/// a file therefore shifted the row indices out from under `selected_ix`: the
/// pane highlighted whichever file now sat at that index, and stepping to a
/// neighbour named a file that might no longer be changed at all.
#[test]
fn a_rescan_re_resolves_an_open_worktree_diff_against_the_new_file_list() {
    use gitcomet_core::domain::{FileStatus, WorktreeDirtySummary};

    let worktree = PathBuf::from("/tmp/wt/a");
    let file = |name: &str| FileStatus {
        path: PathBuf::from(name),
        kind: FileStatusKind::Modified,
        conflict: None,
    };
    let summary = |files: &[&str]| WorktreeDirtySummary {
        path: worktree.clone(),
        head: Some(CommitId("tip".into())),
        branch: Some("side".to_string()),
        detached: false,
        added: 0,
        modified: files.len(),
        deleted: 0,
        staged: Vec::new(),
        unstaged: files.iter().map(|name| file(name)).collect(),
        line_stats: Default::default(),
    };

    let open_on = |state: &mut AppState, files: &[&str], selected_ix: usize| {
        let entries = crate::model::worktree_inline_diff_entries(&summary(files));
        let target = entries[selected_ix].target.clone();
        let repo_state = &mut state.repos[0];
        repo_state.set_worktree_dirty(Loadable::Ready(vec![summary(files)]));
        repo_state.history_state.worktree_selection = Some(worktree.clone());
        repo_state.diff_state.inline_submodule_diff =
            Some(crate::model::InlineSubmoduleDiffState {
                origin: crate::model::ForeignDiffOrigin::Worktree {
                    branch: Some("side".to_string()),
                    detached: false,
                },
                submodule_repo_path: worktree.clone(),
                parent_submodule_path: worktree.clone(),
                entries: entries.into(),
                selected_ix,
                target,
                rev: 1,
                diff_rev: 1,
                diff: Loadable::NotLoaded,
                diff_file_rev: 1,
                diff_file: Loadable::NotLoaded,
                diff_file_image: Loadable::NotLoaded,
            });
    };

    let rescan = |state: &mut AppState, files: &[&str]| {
        let mut repos: RepositoryRuntimes = Default::default();
        let id_alloc = AtomicU64::new(2);
        reduce(
            &mut repos,
            &id_alloc,
            state,
            Msg::Internal(crate::msg::InternalMsg::WorktreeDirtyLoaded {
                repo_id: RepoId(1),
                scope: crate::model::WorktreeDirtyScope::All,
                result: Ok(vec![summary(files)]),
            }),
        );
    };

    let fresh_state = || {
        let mut state = AppState::test_default();
        state.repos.push(RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        ));
        state.active_repo = Some(RepoId(1));
        state
    };

    // The file the diff shows moves down the list: the index has to follow it.
    let mut state = fresh_state();
    open_on(&mut state, &["a.rs", "b.rs", "c.rs"], 2);
    rescan(&mut state, &["b.rs", "c.rs"]);

    let inline = state.repos[0]
        .diff_state
        .inline_submodule_diff
        .as_ref()
        .expect("the diff stays open, its file is still changed");
    assert_eq!(inline.selected_ix, 1);
    assert_eq!(
        inline.entries[inline.selected_ix].path,
        PathBuf::from("c.rs"),
        "the selection must still name the file on screen"
    );
    assert_eq!(
        inline.entries.len(),
        2,
        "the entry list navigation walks must match the rows"
    );

    // The file itself stops being changed: there is no row left to deselect the
    // diff from, so it retires the same way a vanished worktree row does.
    let mut state = fresh_state();
    open_on(&mut state, &["a.rs", "b.rs"], 1);
    rescan(&mut state, &["a.rs"]);
    assert!(
        state.repos[0].diff_state.inline_submodule_diff.is_none(),
        "a diff whose file is no longer changed must not stay open"
    );
}

/// The scan is the only notice a linked worktree's files have changed -- the
/// watcher covers the repo that is open, not the others -- and re-resolving an
/// unmoved selection is a no-op inside `select_inline_submodule_diff`. Without a
/// reload of its own the patch on screen kept its contents from the moment the
/// row was clicked, however many times the file was edited afterwards.
#[test]
fn a_rescan_reloads_the_open_worktree_patch_even_when_nothing_moved() {
    let (mut state, _) = worktree_inline_diff_fixture(&["a.rs"], 0, "side");

    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::WorktreeDirtyLoaded {
            repo_id: RepoId(1),
            scope: crate::model::WorktreeDirtyScope::All,
            result: Ok(vec![worktree_dirty_summary(&["a.rs"], "side")]),
        }),
    );

    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadInlineSubmoduleSelectedDiff { .. })),
        "an identical file list still has to re-read the patch, got {effects:?}"
    );
    let inline = state.repos[0]
        .diff_state
        .inline_submodule_diff
        .as_ref()
        .expect("the diff stays open");
    assert!(
        matches!(inline.diff, Loadable::NotLoaded),
        "a reload of the same target must leave what is on screen alone until \
         the new payload lands, got {:?}",
        inline.diff
    );
}

/// A linked worktree is rescanned on a timer, so re-wrapping an identical scan
/// allocated an entry per changed file on every tick. `Arc::ptr_eq` is how the
/// test sees the list was left alone, not an invariant readers rely on.
#[test]
fn a_rescan_that_moved_nothing_does_not_rebuild_the_worktree_entry_list() {
    let (mut state, _) = worktree_inline_diff_fixture(&["a.rs", "b.rs"], 1, "side");
    let before = Arc::clone(
        &state.repos[0]
            .diff_state
            .inline_submodule_diff
            .as_ref()
            .expect("the diff is open")
            .entries,
    );

    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::WorktreeDirtyLoaded {
            repo_id: RepoId(1),
            scope: crate::model::WorktreeDirtyScope::All,
            result: Ok(vec![worktree_dirty_summary(&["a.rs", "b.rs"], "side")]),
        }),
    );

    let inline = state.repos[0]
        .diff_state
        .inline_submodule_diff
        .as_ref()
        .expect("the diff stays open");
    assert_eq!(inline.selected_ix, 1);
    assert!(
        Arc::ptr_eq(&before, &inline.entries),
        "an unchanged scan must not rebuild the list"
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::WorktreeDirtyLoaded {
            repo_id: RepoId(1),
            scope: crate::model::WorktreeDirtyScope::All,
            result: Ok(vec![worktree_dirty_summary(
                &["a.rs", "b.rs", "c.rs"],
                "side",
            )]),
        }),
    );
    let inline = state.repos[0]
        .diff_state
        .inline_submodule_diff
        .as_ref()
        .expect("the diff stays open");
    assert_eq!(inline.entries.len(), 3, "a new file joins the list");
    assert_eq!(inline.selected_ix, 1, "and the open file keeps its row");
    assert!(
        !Arc::ptr_eq(&before, &inline.entries),
        "a scan that did move something must replace the list"
    );
}

/// A file that is staged *and* modified again has a row in each half of the
/// list. Re-resolving by path alone always found the staged one, so every rescan
/// silently moved a reader of the unstaged half onto the staged copy.
#[test]
fn a_rescan_keeps_the_worktree_diff_on_the_half_it_was_opened_from() {
    use gitcomet_core::domain::{DiffArea, FileStatus, WorktreeDirtySummary};

    let worktree = PathBuf::from("/tmp/wt/a");
    let file = FileStatus {
        path: PathBuf::from("both.rs"),
        kind: FileStatusKind::Modified,
        conflict: None,
    };
    let summary = || WorktreeDirtySummary {
        path: worktree.clone(),
        head: Some(CommitId("tip".into())),
        branch: Some("side".to_string()),
        detached: false,
        added: 0,
        modified: 1,
        deleted: 0,
        staged: vec![file.clone()],
        unstaged: vec![file.clone()],
        line_stats: Default::default(),
    };

    let entries = crate::model::worktree_inline_diff_entries(&summary());
    assert_eq!(entries.len(), 2, "the file has a row in each half");
    let unstaged_ix = 1;
    let target = entries[unstaged_ix].target.clone();
    assert!(matches!(
        &target,
        gitcomet_core::domain::DiffTarget::WorkingTree {
            area: DiffArea::Unstaged,
            ..
        }
    ));

    let mut state = AppState::test_default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.set_worktree_dirty(Loadable::Ready(vec![summary()]));
    repo_state.history_state.worktree_selection = Some(worktree.clone());
    repo_state.diff_state.inline_submodule_diff = Some(crate::model::InlineSubmoduleDiffState {
        origin: crate::model::ForeignDiffOrigin::Worktree {
            branch: Some("side".to_string()),
            detached: false,
        },
        submodule_repo_path: worktree.clone(),
        parent_submodule_path: worktree.clone(),
        entries: entries.into(),
        selected_ix: unstaged_ix,
        target,
        rev: 1,
        diff_rev: 1,
        diff: Loadable::NotLoaded,
        diff_file_rev: 1,
        diff_file: Loadable::NotLoaded,
        diff_file_image: Loadable::NotLoaded,
    });
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));

    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::WorktreeDirtyLoaded {
            repo_id: RepoId(1),
            scope: crate::model::WorktreeDirtyScope::All,
            result: Ok(vec![summary()]),
        }),
    );

    let inline = state.repos[0]
        .diff_state
        .inline_submodule_diff
        .as_ref()
        .expect("the diff stays open");
    assert_eq!(inline.selected_ix, unstaged_ix);
    assert!(
        matches!(
            &inline.target,
            gitcomet_core::domain::DiffTarget::WorkingTree {
                area: DiffArea::Unstaged,
                ..
            }
        ),
        "the rescan must not swap the reader onto the staged copy, got {:?}",
        inline.target
    );
}

/// The chip over the diff is labelled from `origin`, captured when the row was
/// clicked. A checkout inside that worktree moves the branch under it, and with
/// an unchanged file set nothing else in the refresh runs at all.
#[test]
fn a_rescan_refreshes_the_branch_the_worktree_diff_is_labelled_with() {
    let (mut state, _) = worktree_inline_diff_fixture(&["a.rs"], 0, "side");

    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::WorktreeDirtyLoaded {
            repo_id: RepoId(1),
            scope: crate::model::WorktreeDirtyScope::All,
            result: Ok(vec![worktree_dirty_summary(&["a.rs"], "other")]),
        }),
    );

    let inline = state.repos[0]
        .diff_state
        .inline_submodule_diff
        .as_ref()
        .expect("the diff stays open");
    assert_eq!(
        inline.origin,
        crate::model::ForeignDiffOrigin::Worktree {
            branch: Some("other".to_string()),
            detached: false,
        },
        "the chip must name the branch the worktree is on now"
    );
}

/// A pending history reveal re-drives on every render of the history panel, so
/// this message arrives once per frame until pagination reaches its target.
/// Re-running the body each time bumped `commit_details_rev` -- which the details
/// pane hashes, so the repaint drove the next render -- and re-armed a full
/// `git status` walk across every linked worktree.
#[test]
fn reselecting_the_same_worktree_row_is_a_no_op() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let path = PathBuf::from("/tmp/wt/a");
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.open = Loadable::Ready(());
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));

    let select = || Msg::SelectWorktreeUncommitted {
        request_id: None,
        repo_id: RepoId(1),
        path: path.clone(),
    };
    reduce(&mut repos, &id_alloc, &mut state, select());

    let details_rev = state.repos[0].history_state.commit_details_rev;
    let selection_rev = state.repos[0].history_state.worktree_selection_rev;
    let effects = reduce(&mut repos, &id_alloc, &mut state, select());

    assert!(
        effects.is_empty(),
        "a repeat must not re-arm the scan, got {effects:?}"
    );
    assert_eq!(
        state.repos[0].history_state.commit_details_rev, details_rev,
        "a repeat must not force the details pane to repaint"
    );
    assert_eq!(
        state.repos[0].history_state.worktree_selection_rev,
        selection_rev
    );
}

/// A worktree selection ends in more ways than it begins, and every one of them
/// leaves the worktree's inline diff without a row to deselect it. The invariant
/// runs after each message, so all of these exits retire it.
#[test]
fn every_way_out_of_a_worktree_selection_retires_its_inline_diff() {
    use gitcomet_core::domain::WorktreeDirtySummary;

    let worktree = PathBuf::from("/tmp/wt/a");
    let inline_target = gitcomet_core::domain::DiffTarget::working_tree(
        PathBuf::from("src/lib.rs"),
        gitcomet_core::domain::DiffArea::Unstaged,
    );
    let state_with_open_worktree_diff = || {
        let mut state = AppState::test_default();
        let mut repo_state = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        );
        repo_state.history_state.worktree_selection = Some(worktree.clone());
        repo_state.diff_state.inline_submodule_diff =
            Some(crate::model::InlineSubmoduleDiffState {
                origin: crate::model::ForeignDiffOrigin::Worktree {
                    branch: Some("side".to_string()),
                    detached: false,
                },
                submodule_repo_path: worktree.clone(),
                parent_submodule_path: worktree.clone(),
                entries: vec![crate::model::InlineSubmoduleDiffEntry {
                    path: PathBuf::from("src/lib.rs"),
                    kind: FileStatusKind::Modified,
                    target: inline_target.clone(),
                    section: crate::model::InlineSubmoduleDiffSection::LiveUnstaged,
                }]
                .into(),
                selected_ix: 0,
                target: inline_target.clone(),
                rev: 1,
                diff_rev: 1,
                diff: Loadable::NotLoaded,
                diff_file_rev: 1,
                diff_file: Loadable::NotLoaded,
                diff_file_image: Loadable::NotLoaded,
            });
        state.repos.push(repo_state);
        state.active_repo = Some(RepoId(1));
        state
    };

    let exits: Vec<(&str, Msg)> = vec![
        (
            "selecting a commit",
            Msg::SelectCommit {
                request_id: None,
                repo_id: RepoId(1),
                commit_id: CommitId("tip".into()),
            },
        ),
        (
            "clearing the commit selection",
            Msg::ClearCommitSelection {
                request_id: None,
                repo_id: RepoId(1),
            },
        ),
        (
            "a scan that no longer lists the worktree",
            Msg::Internal(crate::msg::InternalMsg::WorktreeDirtyLoaded {
                repo_id: RepoId(1),
                scope: crate::model::WorktreeDirtyScope::All,
                result: Ok(Vec::<WorktreeDirtySummary>::new()),
            }),
        ),
        (
            "switching to another worktree",
            Msg::SelectWorktreeUncommitted {
                request_id: None,
                repo_id: RepoId(1),
                path: PathBuf::from("/tmp/wt/b"),
            },
        ),
    ];

    for (label, msg) in exits {
        let mut repos: RepositoryRuntimes = Default::default();
        let id_alloc = AtomicU64::new(2);
        let mut state = state_with_open_worktree_diff();

        reduce(&mut repos, &id_alloc, &mut state, msg);

        assert!(
            state.repos[0].diff_state.inline_submodule_diff.is_none(),
            "{label} must retire the worktree's inline diff"
        );
    }

    // The counterpart: a message that leaves the selection intact leaves the diff
    // intact too, or every unrelated refresh would close the file being read.
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = state_with_open_worktree_diff();
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectWorktreeUncommitted {
            request_id: None,
            repo_id: RepoId(1),
            path: worktree.clone(),
        },
    );
    assert!(
        state.repos[0].diff_state.inline_submodule_diff.is_some(),
        "re-selecting the same worktree must keep its open diff"
    );
}

/// Only the selected worktree's changed files live in state, so selecting a row
/// has to ask for a scan that carries them — and that scan has to name the
/// worktree that was just selected, not whichever one was selected before.
#[test]
fn selecting_a_worktree_requests_a_scan_for_its_own_files() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.open = Loadable::Ready(());
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));

    let selected = PathBuf::from("/tmp/wt/a");
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectWorktreeUncommitted {
            request_id: None,
            repo_id: RepoId(1),
            path: selected.clone(),
        },
    );

    let files_for = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::LoadWorktreeDirty { files_for, .. } => Some(files_for.clone()),
            _ => None,
        })
        .expect("selecting a worktree should request a scan");
    assert_eq!(
        files_for,
        Some(selected),
        "the scan must carry the files of the worktree just selected"
    );
}

/// The scan is also triggered by the watcher and by window focus, and those
/// carry whatever is selected at the time — including nothing.
#[test]
fn a_scan_with_no_worktree_selected_asks_for_counts_alone() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.open = Loadable::Ready(());
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadWorktreeDirty { repo_id: RepoId(1) },
    );

    let files_for = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::LoadWorktreeDirty { files_for, .. } => Some(files_for.clone()),
            _ => None,
        })
        .expect("the refresh should request a scan");
    assert_eq!(
        files_for, None,
        "with no row selected there is no file list worth carrying"
    );
}
