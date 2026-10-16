use super::*;

#[test]
fn select_diff_for_checked_out_submodule_marker_loads_summary_before_submodules_load() {
    let dir = tempfile::tempdir().expect("tempdir");
    let submodule_path = PathBuf::from("vendor/submodule");
    std::fs::create_dir_all(dir.path().join(&submodule_path)).expect("submodule directory");
    std::fs::write(
        dir.path().join(&submodule_path).join(".git"),
        "gitdir: ../../.git/modules/vendor/submodule\n",
    )
    .expect("submodule gitdir marker");

    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: dir.path().to_path_buf(),
        },
    );
    let target = gitcomet_core::domain::DiffTarget::working_tree(
        submodule_path.clone(),
        gitcomet_core::domain::DiffArea::Unstaged,
    );
    repo_state.set_status(Loadable::Ready(Arc::new(RepoStatus {
        unstaged: std::sync::Arc::new(vec![FileStatus {
            path: submodule_path,
            kind: FileStatusKind::Modified,
            conflict: None,
        }]),
        staged: std::sync::Arc::new(vec![]),
    })));
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target,
        },
    );

    let repo_state = state.repos.first().expect("repo state to exist");
    assert!(matches!(repo_state.diff_state.diff, Loadable::NotLoaded));
    assert!(matches!(
        repo_state.diff_state.diff_file,
        Loadable::NotLoaded
    ));
    assert!(repo_state.diff_state.submodule_summary.is_loading());
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadSelectedDiff {
            repo_id: RepoId(1),
            load_patch_diff: false,
            load_file_text: false,
            preview_text_side: None,
            load_file_image: false,
            load_submodule_summary: true,
        }]
    ));
}

#[test]
fn open_inline_submodule_diff_loads_patch_and_file_text_for_text_targets() {
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

    let target = gitcomet_core::domain::DiffTarget::commit_range(
        CommitId("aaaa".into()),
        Some(CommitId("bbbb".into())),
        Some(PathBuf::from("src/lib.rs")),
    );
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenInlineSubmoduleDiff {
            origin: crate::model::ForeignDiffOrigin::Submodule,
            repo_id: RepoId(1),
            submodule_repo_path: PathBuf::from("/tmp/repo/vendor/submodule"),
            parent_submodule_path: PathBuf::from("vendor/submodule"),
            entries: vec![crate::model::InlineSubmoduleDiffEntry {
                path: PathBuf::from("src/lib.rs"),
                kind: FileStatusKind::Modified,
                target: target.clone(),
                section: crate::model::InlineSubmoduleDiffSection::Range(
                    gitcomet_core::domain::SubmoduleDiffRangeKind::CommitHistory,
                ),
            }]
            .into(),
            selected_ix: 0,
        },
    );

    let inline = state
        .repos
        .first()
        .and_then(|repo| repo.diff_state.inline_submodule_diff.as_ref())
        .expect("inline submodule diff should be open");
    assert_eq!(inline.target, target);
    assert!(inline.diff.is_loading());
    assert!(inline.diff_file.is_loading());
    assert!(matches!(inline.diff_file_image, Loadable::NotLoaded));
    assert!(matches!(
        effects.as_slice(),
        [
            Effect::LoadInlineSubmoduleSelectedDiff {
                repo_id: RepoId(1),
                inline_rev: 1,
            },
            Effect::LoadInlineSubmoduleSelectedDiffFile {
                repo_id: RepoId(1),
                inline_rev: 1,
            },
        ]
    ));
}

#[test]
fn open_inline_submodule_diff_loads_patch_file_and_image_for_svg_targets() {
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

    let target = gitcomet_core::domain::DiffTarget::working_tree(
        PathBuf::from("icons/logo.svg"),
        gitcomet_core::domain::DiffArea::Unstaged,
    );
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenInlineSubmoduleDiff {
            origin: crate::model::ForeignDiffOrigin::Submodule,
            repo_id: RepoId(1),
            submodule_repo_path: PathBuf::from("/tmp/repo/vendor/submodule"),
            parent_submodule_path: PathBuf::from("vendor/submodule"),
            entries: vec![crate::model::InlineSubmoduleDiffEntry {
                path: PathBuf::from("icons/logo.svg"),
                kind: FileStatusKind::Modified,
                target: target.clone(),
                section: crate::model::InlineSubmoduleDiffSection::LiveUnstaged,
            }]
            .into(),
            selected_ix: 0,
        },
    );

    let inline = state
        .repos
        .first()
        .and_then(|repo| repo.diff_state.inline_submodule_diff.as_ref())
        .expect("inline submodule diff should be open");
    assert_eq!(inline.target, target);
    assert!(inline.diff.is_loading());
    assert!(inline.diff_file.is_loading());
    assert!(inline.diff_file_image.is_loading());
    assert!(matches!(
        effects.as_slice(),
        [
            Effect::LoadInlineSubmoduleSelectedDiff {
                repo_id: RepoId(1),
                inline_rev: 1,
            },
            Effect::LoadInlineSubmoduleSelectedDiffFile {
                repo_id: RepoId(1),
                inline_rev: 1,
            },
            Effect::LoadInlineSubmoduleSelectedDiffFileImage {
                repo_id: RepoId(1),
                inline_rev: 1,
            },
        ]
    ));
}

#[test]
fn stale_inline_submodule_file_load_is_ignored() {
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

    let target = gitcomet_core::domain::DiffTarget::commit_range(
        CommitId("aaaa".into()),
        Some(CommitId("bbbb".into())),
        Some(PathBuf::from("src/lib.rs")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenInlineSubmoduleDiff {
            origin: crate::model::ForeignDiffOrigin::Submodule,
            repo_id: RepoId(1),
            submodule_repo_path: PathBuf::from("/tmp/repo/vendor/submodule"),
            parent_submodule_path: PathBuf::from("vendor/submodule"),
            entries: vec![crate::model::InlineSubmoduleDiffEntry {
                path: PathBuf::from("src/lib.rs"),
                kind: FileStatusKind::Modified,
                target: target.clone(),
                section: crate::model::InlineSubmoduleDiffSection::Range(
                    gitcomet_core::domain::SubmoduleDiffRangeKind::CommitHistory,
                ),
            }]
            .into(),
            selected_ix: 0,
        },
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::InlineSubmoduleDiffFileLoaded {
            repo_id: RepoId(1),
            inline_rev: 0,
            target: target.clone(),
            result: Ok(Some(gitcomet_core::domain::FileDiffText::new(
                PathBuf::from("src/lib.rs"),
                Some("before\n".to_string()),
                Some("after\n".to_string()),
            ))),
        }),
    );

    let inline = state
        .repos
        .first()
        .and_then(|repo| repo.diff_state.inline_submodule_diff.as_ref())
        .expect("inline submodule diff should remain open");
    assert!(effects.is_empty());
    assert!(inline.diff_file.is_loading());
    assert_eq!(inline.diff_file_rev, 0);
}

#[test]
fn stale_inline_submodule_file_load_after_reopen_is_ignored() {
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

    let target = gitcomet_core::domain::DiffTarget::commit_range(
        CommitId("aaaa".into()),
        Some(CommitId("bbbb".into())),
        Some(PathBuf::from("src/lib.rs")),
    );
    let entry = crate::model::InlineSubmoduleDiffEntry {
        path: PathBuf::from("src/lib.rs"),
        kind: FileStatusKind::Modified,
        target: target.clone(),
        section: crate::model::InlineSubmoduleDiffSection::Range(
            gitcomet_core::domain::SubmoduleDiffRangeKind::CommitHistory,
        ),
    };

    let first_effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenInlineSubmoduleDiff {
            origin: crate::model::ForeignDiffOrigin::Submodule,
            repo_id: RepoId(1),
            submodule_repo_path: PathBuf::from("/tmp/repo/vendor/first"),
            parent_submodule_path: PathBuf::from("vendor/first"),
            entries: vec![entry.clone()].into(),
            selected_ix: 0,
        },
    );
    assert!(matches!(
        first_effects.as_slice(),
        [
            Effect::LoadInlineSubmoduleSelectedDiff {
                repo_id: RepoId(1),
                inline_rev: 1,
            },
            Effect::LoadInlineSubmoduleSelectedDiffFile {
                repo_id: RepoId(1),
                inline_rev: 1,
            },
        ]
    ));

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseInlineSubmoduleDiff { repo_id: RepoId(1) },
    );
    let second_effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenInlineSubmoduleDiff {
            origin: crate::model::ForeignDiffOrigin::Submodule,
            repo_id: RepoId(1),
            submodule_repo_path: PathBuf::from("/tmp/repo/vendor/second"),
            parent_submodule_path: PathBuf::from("vendor/second"),
            entries: vec![entry].into(),
            selected_ix: 0,
        },
    );
    assert!(matches!(
        second_effects.as_slice(),
        [
            Effect::LoadInlineSubmoduleSelectedDiff {
                repo_id: RepoId(1),
                inline_rev: 2,
            },
            Effect::LoadInlineSubmoduleSelectedDiffFile {
                repo_id: RepoId(1),
                inline_rev: 2,
            },
        ]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::InlineSubmoduleDiffFileLoaded {
            repo_id: RepoId(1),
            inline_rev: 1,
            target: target.clone(),
            result: Ok(Some(gitcomet_core::domain::FileDiffText::new(
                PathBuf::from("src/lib.rs"),
                Some("before\n".to_string()),
                Some("after\n".to_string()),
            ))),
        }),
    );

    let inline = state
        .repos
        .first()
        .and_then(|repo| repo.diff_state.inline_submodule_diff.as_ref())
        .expect("inline submodule diff should remain open");
    assert!(effects.is_empty());
    assert_eq!(
        inline.submodule_repo_path,
        PathBuf::from("/tmp/repo/vendor/second")
    );
    assert_eq!(inline.rev, 2);
    assert!(inline.diff_file.is_loading());
    assert_eq!(inline.diff_file_rev, 0);
}

#[test]
fn submodule_summary_refresh_reloads_open_inline_diff_when_selected_target_remains() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let parent_path = PathBuf::from("vendor/submodule");
    let parent_target = DiffTarget::working_tree(parent_path.clone(), DiffArea::Unstaged);
    let inline_target = DiffTarget::working_tree(PathBuf::from("src/lib.rs"), DiffArea::Unstaged);
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.set_status(Loadable::Ready(Arc::new(RepoStatus {
        unstaged: std::sync::Arc::new(vec![FileStatus {
            path: parent_path.clone(),
            kind: FileStatusKind::Modified,
            conflict: None,
        }]),
        staged: std::sync::Arc::new(vec![]),
    })));
    repo_state.set_submodules(Loadable::Ready(vec![Submodule {
        path: parent_path.clone(),
        recorded_head: CommitId("old-recorded".into()),
        checked_out_head: Some(CommitId("old-head".into())),
        status: SubmoduleStatus::HeadMismatch,
    }]));
    repo_state.set_diff_target(Some(parent_target.clone()));
    repo_state.diff_state.submodule_summary = Loadable::Ready(Arc::new(SubmoduleDiffSummary {
        path: parent_path.clone(),
        mode: SubmoduleDiffSummaryMode::Worktree,
        status: Some(SubmoduleStatus::HeadMismatch),
        checkout_available: true,
        commit_id: None,
        parent_commit_id: None,
        checked_out_head: Some(CommitId("old-head".into())),
        ranges: vec![],
        live_staged: vec![],
        live_unstaged: vec![
            SubmoduleInnerChange::new(PathBuf::from("src/lib.rs"), FileStatusKind::Modified)
                .with_line_counts(Some(1), Some(1)),
        ],
    }));
    repo_state.diff_state.inline_submodule_diff_rev = 1;
    repo_state.diff_state.inline_submodule_diff = Some(crate::model::InlineSubmoduleDiffState {
        origin: crate::model::ForeignDiffOrigin::Submodule,
        submodule_repo_path: PathBuf::from("/tmp/repo/vendor/submodule"),
        parent_submodule_path: parent_path.clone(),
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
        diff: Loadable::Ready(Arc::new(gitcomet_core::domain::Diff {
            target: inline_target.clone(),
            lines: Vec::new(),
        })),
        diff_file_rev: 1,
        diff_file: Loadable::Ready(Some(Arc::new(gitcomet_core::domain::FileDiffText::new(
            PathBuf::from("src/lib.rs"),
            Some("before\n".to_string()),
            Some("after\n".to_string()),
        )))),
        diff_file_image: Loadable::NotLoaded,
    });
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::SubmoduleSummaryLoaded {
            repo_id: RepoId(1),
            target: parent_target,
            result: Ok(SubmoduleDiffSummary {
                path: parent_path,
                mode: SubmoduleDiffSummaryMode::Worktree,
                status: Some(SubmoduleStatus::UpToDate),
                checkout_available: true,
                commit_id: None,
                parent_commit_id: None,
                checked_out_head: Some(CommitId("new-head".into())),
                ranges: vec![],
                live_staged: vec![],
                live_unstaged: vec![
                    SubmoduleInnerChange::new(PathBuf::from("README.md"), FileStatusKind::Modified)
                        .with_line_counts(Some(2), Some(0)),
                    SubmoduleInnerChange::new(
                        PathBuf::from("src/lib.rs"),
                        FileStatusKind::Modified,
                    )
                    .with_line_counts(Some(4), Some(1)),
                ],
            }),
        }),
    );

    let inline = state
        .repos
        .first()
        .and_then(|repo| repo.diff_state.inline_submodule_diff.as_ref())
        .expect("inline submodule diff should remain open");
    assert_eq!(inline.rev, 2);
    assert_eq!(inline.selected_ix, 1);
    assert_eq!(inline.entries.len(), 2);
    assert!(inline.diff.is_loading());
    assert!(inline.diff_file.is_loading());
    assert!(matches!(inline.diff_file_image, Loadable::NotLoaded));
    assert!(matches!(
        effects.as_slice(),
        [
            Effect::LoadInlineSubmoduleSelectedDiff {
                repo_id: RepoId(1),
                inline_rev: 2,
            },
            Effect::LoadInlineSubmoduleSelectedDiffFile {
                repo_id: RepoId(1),
                inline_rev: 2,
            },
        ]
    ));
}

#[test]
fn status_actions_retire_inline_submodule_views_only_with_the_parent_entry() {
    use gitcomet_core::domain::DiffArea;
    for area in [DiffArea::Unstaged, DiffArea::Staged] {
        for paths in [
            vec![],
            vec![PathBuf::from("module")],
            vec![PathBuf::from("inner.rs")],
        ] {
            for succeeds in [false, true] {
                let target = DiffTarget::working_tree("module".into(), area);
                let mut state = status_action_state(target.clone());
                reduce(
                    &mut Default::default(),
                    &AtomicU64::new(3),
                    &mut state,
                    Msg::OpenInlineSubmoduleDiff {
                        repo_id: RepoId(1),
                        origin: crate::model::ForeignDiffOrigin::Submodule,
                        submodule_repo_path: "/tmp/repo/module".into(),
                        parent_submodule_path: "module".into(),
                        entries: vec![crate::model::InlineSubmoduleDiffEntry {
                            path: "inner.rs".into(),
                            kind: FileStatusKind::Modified,
                            target: DiffTarget::working_tree("inner.rs".into(), DiffArea::Unstaged),
                            section: crate::model::InlineSubmoduleDiffSection::LiveUnstaged,
                        }]
                        .into(),
                        selected_ix: 0,
                    },
                );
                finish_status_action(
                    &mut state,
                    RepoId(1),
                    RepoActionKind::StagePaths,
                    paths.clone(),
                    succeeds,
                );
                let retired = succeeds
                    && area == DiffArea::Unstaged
                    && (paths.is_empty() || paths.contains(&PathBuf::from("module")));
                assert_eq!(
                    state.repos[0].diff_state.diff_target,
                    (!retired).then_some(target)
                );
                assert_eq!(
                    state.repos[0].diff_state.inline_submodule_diff.is_none(),
                    retired
                );
            }
        }
    }
}

/// A submodule inline diff has no worktree row behind it, so the worktree
/// invariant must not touch it.
#[test]
fn a_submodule_inline_diff_survives_the_worktree_invariant() {
    let submodule_path = PathBuf::from("/tmp/repo/vendor/submodule");
    let inline_target = gitcomet_core::domain::DiffTarget::working_tree(
        PathBuf::from("src/lib.rs"),
        gitcomet_core::domain::DiffArea::Unstaged,
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
    repo_state.diff_state.inline_submodule_diff = Some(crate::model::InlineSubmoduleDiffState {
        origin: crate::model::ForeignDiffOrigin::Submodule,
        submodule_repo_path: submodule_path.clone(),
        parent_submodule_path: submodule_path.clone(),
        entries: Vec::new().into(),
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

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ClearCommitSelection {
            request_id: None,
            repo_id: RepoId(1),
        },
    );

    assert!(
        state.repos[0].diff_state.inline_submodule_diff.is_some(),
        "a submodule diff has no worktree selection and must be left alone"
    );
}

/// `Arc::make_mut` re-clones the state this lands in on every later dispatch,
/// so the reducer must share the caller's list rather than copy it.
#[test]
fn open_inline_submodule_diff_shares_the_callers_entry_list() {
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

    let target = gitcomet_core::domain::DiffTarget::commit_range(
        CommitId("aaaa".into()),
        Some(CommitId("bbbb".into())),
        Some(PathBuf::from("src/lib.rs")),
    );
    let entries: Arc<[crate::model::InlineSubmoduleDiffEntry]> = (0..512)
        .map(|ix| crate::model::InlineSubmoduleDiffEntry {
            path: PathBuf::from(format!("src/file_{ix}.rs")),
            kind: FileStatusKind::Modified,
            target: target.clone(),
            section: crate::model::InlineSubmoduleDiffSection::Range(
                gitcomet_core::domain::SubmoduleDiffRangeKind::CommitHistory,
            ),
        })
        .collect();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenInlineSubmoduleDiff {
            origin: crate::model::ForeignDiffOrigin::Submodule,
            repo_id: RepoId(1),
            submodule_repo_path: PathBuf::from("/tmp/repo/vendor/submodule"),
            parent_submodule_path: PathBuf::from("vendor/submodule"),
            entries: Arc::clone(&entries),
            selected_ix: 3,
        },
    );

    let inline = state.repos[0]
        .diff_state
        .inline_submodule_diff
        .as_ref()
        .expect("inline submodule diff should be open");
    assert_eq!(inline.selected_ix, 3);
    assert!(
        Arc::ptr_eq(&entries, &inline.entries),
        "the reducer must store the caller's entries, not a copy"
    );
}

/// The UI caches one prepared row per changed file against this `Arc` and rev,
/// so a reload that produced the same summary must not invalidate either.
#[test]
fn reloading_an_identical_submodule_summary_keeps_the_arc_and_revision() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let path = PathBuf::from("vendor/submodule");
    let target = gitcomet_core::domain::DiffTarget::working_tree(path.clone(), DiffArea::Unstaged);
    let summary = || SubmoduleDiffSummary {
        path: path.clone(),
        mode: SubmoduleDiffSummaryMode::Worktree,
        status: Some(SubmoduleStatus::HeadMismatch),
        checkout_available: true,
        commit_id: None,
        parent_commit_id: None,
        checked_out_head: Some(CommitId("head".into())),
        ranges: vec![],
        live_staged: vec![],
        live_unstaged: vec![
            SubmoduleInnerChange::new(PathBuf::from("src/lib.rs"), FileStatusKind::Modified)
                .with_line_counts(Some(1), Some(1)),
        ],
    };

    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.set_status(Loadable::Ready(Arc::new(RepoStatus {
        unstaged: std::sync::Arc::new(vec![FileStatus {
            path: path.clone(),
            kind: FileStatusKind::Modified,
            conflict: None,
        }]),
        staged: std::sync::Arc::new(vec![]),
    })));
    repo_state.set_submodules(Loadable::Ready(vec![Submodule {
        path: path.clone(),
        recorded_head: CommitId("recorded".into()),
        checked_out_head: Some(CommitId("head".into())),
        status: SubmoduleStatus::HeadMismatch,
    }]));
    repo_state.set_diff_target(Some(target.clone()));
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));

    let load = |state: &mut AppState, repos: &mut RepositoryRuntimes| {
        reduce(
            repos,
            &id_alloc,
            state,
            Msg::Internal(crate::msg::InternalMsg::SubmoduleSummaryLoaded {
                repo_id: RepoId(1),
                target: target.clone(),
                result: Ok(summary()),
            }),
        );
    };
    let snapshot = |state: &AppState| {
        let diff_state = &state.repos[0].diff_state;
        let Loadable::Ready(summary) = &diff_state.submodule_summary else {
            panic!("summary should be ready");
        };
        (Arc::clone(summary), diff_state.submodule_summary_rev)
    };

    load(&mut state, &mut repos);
    let (first, first_rev) = snapshot(&state);

    load(&mut state, &mut repos);
    let (second, second_rev) = snapshot(&state);
    assert!(
        Arc::ptr_eq(&first, &second),
        "an identical reload must keep the same Arc"
    );
    assert_eq!(first_rev, second_rev, "and the same revision");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::SubmoduleSummaryLoaded {
            repo_id: RepoId(1),
            target: target.clone(),
            result: Ok(SubmoduleDiffSummary {
                status: Some(SubmoduleStatus::UpToDate),
                ..summary()
            }),
        }),
    );
    let (third, third_rev) = snapshot(&state);
    assert!(
        !Arc::ptr_eq(&first, &third),
        "a changed summary replaces it"
    );
    assert_ne!(first_rev, third_rev, "and bumps the revision");
}

/// The poll behind an idle repository redelivers the same summary every 250ms:
/// rebuilding the entry list from it is churn, but re-reading the patch is not
/// -- editing an inner file in place moves nothing in the summary.
#[test]
fn reloading_an_identical_submodule_summary_rereads_the_patch_without_rebuilding_entries() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let path = PathBuf::from("vendor/submodule");
    let target = gitcomet_core::domain::DiffTarget::working_tree(path.clone(), DiffArea::Unstaged);
    let summary = || SubmoduleDiffSummary {
        path: path.clone(),
        mode: SubmoduleDiffSummaryMode::Worktree,
        status: Some(SubmoduleStatus::HeadMismatch),
        checkout_available: true,
        commit_id: None,
        parent_commit_id: None,
        checked_out_head: Some(CommitId("head".into())),
        ranges: vec![],
        live_staged: vec![],
        live_unstaged: (0..4)
            .map(|ix| {
                SubmoduleInnerChange::new(
                    PathBuf::from(format!("src/file_{ix}.rs")),
                    FileStatusKind::Modified,
                )
                .with_line_counts(Some(1), Some(1))
            })
            .collect(),
    };
    let entries: Arc<[crate::model::InlineSubmoduleDiffEntry]> =
        crate::model::submodule_inline_diff_entries(&summary()).into();
    let inline_target = entries[2].target.clone();

    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.set_status(Loadable::Ready(Arc::new(RepoStatus {
        unstaged: std::sync::Arc::new(vec![FileStatus {
            path: path.clone(),
            kind: FileStatusKind::Modified,
            conflict: None,
        }]),
        staged: std::sync::Arc::new(vec![]),
    })));
    repo_state.set_submodules(Loadable::Ready(vec![Submodule {
        path: path.clone(),
        recorded_head: CommitId("recorded".into()),
        checked_out_head: Some(CommitId("head".into())),
        status: SubmoduleStatus::HeadMismatch,
    }]));
    repo_state.set_diff_target(Some(target.clone()));
    repo_state.diff_state.submodule_summary = Loadable::Ready(Arc::new(summary()));
    repo_state.diff_state.submodule_summary_rev = 1;
    repo_state.diff_state.inline_submodule_diff = Some(crate::model::InlineSubmoduleDiffState {
        origin: crate::model::ForeignDiffOrigin::Submodule,
        submodule_repo_path: PathBuf::from("/tmp/repo/vendor/submodule"),
        parent_submodule_path: path.clone(),
        entries: Arc::clone(&entries),
        selected_ix: 2,
        target: inline_target.clone(),
        rev: 7,
        diff_rev: 1,
        diff: Loadable::Ready(Arc::new(gitcomet_core::domain::Diff {
            target: inline_target,
            lines: Vec::new(),
        })),
        diff_file_rev: 1,
        diff_file: Loadable::NotLoaded,
        diff_file_image: Loadable::NotLoaded,
    });
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));
    let diff_state_rev = state.repos[0].diff_state.diff_state_rev;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::SubmoduleSummaryLoaded {
            repo_id: RepoId(1),
            target: target.clone(),
            result: Ok(summary()),
        }),
    );

    assert!(
        matches!(
            effects.as_slice(),
            [
                Effect::LoadInlineSubmoduleSelectedDiff { .. },
                Effect::LoadInlineSubmoduleSelectedDiffFile { .. },
            ]
        ),
        "the open live file must still be re-read, got {effects:?}"
    );
    let inline = state.repos[0]
        .diff_state
        .inline_submodule_diff
        .as_ref()
        .expect("the inline diff stays open");
    assert_eq!(inline.selected_ix, 2, "the selection must not move");
    assert!(
        Arc::ptr_eq(&entries, &inline.entries),
        "and its entry list must not be rebuilt"
    );
    assert!(
        matches!(inline.diff, Loadable::Ready(_)),
        "the patch on screen must stay until the new one lands, got {:?}",
        inline.diff
    );
    assert_eq!(
        state.repos[0].diff_state.submodule_summary_rev, 1,
        "the summary itself did not change"
    );
    assert_eq!(
        state.repos[0].diff_state.diff_state_rev, diff_state_rev,
        "and the re-read owes no repaint of its own"
    );

    // A changed summary still refreshes the open diff.
    let mut changed = summary();
    changed.live_unstaged.remove(0);
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::SubmoduleSummaryLoaded {
            repo_id: RepoId(1),
            target,
            result: Ok(changed),
        }),
    );
    let inline = state.repos[0]
        .diff_state
        .inline_submodule_diff
        .as_ref()
        .expect("the inline diff stays open");
    assert_eq!(inline.selected_ix, 1, "the selected file moved up one row");
    assert_ne!(inline.rev, 7);
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadInlineSubmoduleSelectedDiff { .. })),
        "a changed summary must still reload the open diff, got {effects:?}"
    );
}

/// The other half of the same rule: a range entry is a diff between two fixed
/// commits, so an identical summary has nothing to re-read for it.
#[test]
fn reloading_an_identical_submodule_summary_does_not_re_read_a_range_entry() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let path = PathBuf::from("vendor/submodule");
    let target = gitcomet_core::domain::DiffTarget::working_tree(path.clone(), DiffArea::Unstaged);
    let summary = || SubmoduleDiffSummary {
        path: path.clone(),
        mode: SubmoduleDiffSummaryMode::Worktree,
        status: Some(SubmoduleStatus::HeadMismatch),
        checkout_available: true,
        commit_id: None,
        parent_commit_id: None,
        checked_out_head: Some(CommitId("head".into())),
        ranges: vec![gitcomet_core::domain::SubmoduleDiffRange {
            kind: gitcomet_core::domain::SubmoduleDiffRangeKind::StagedPointer,
            from: Some(CommitId("aaaa".into())),
            to: Some(CommitId("bbbb".into())),
            unavailable_reason: None,
            changes: vec![
                SubmoduleInnerChange::new(PathBuf::from("src/lib.rs"), FileStatusKind::Modified)
                    .with_line_counts(Some(1), Some(1)),
            ],
        }],
        live_staged: vec![],
        live_unstaged: vec![],
    };
    let entries: Arc<[crate::model::InlineSubmoduleDiffEntry]> =
        crate::model::submodule_inline_diff_entries(&summary()).into();
    let inline_target = entries[0].target.clone();
    assert!(matches!(
        inline_target,
        gitcomet_core::domain::DiffTarget::CommitRange { .. }
    ));

    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.set_status(Loadable::Ready(Arc::new(RepoStatus {
        unstaged: std::sync::Arc::new(vec![FileStatus {
            path: path.clone(),
            kind: FileStatusKind::Modified,
            conflict: None,
        }]),
        staged: std::sync::Arc::new(vec![]),
    })));
    repo_state.set_submodules(Loadable::Ready(vec![Submodule {
        path: path.clone(),
        recorded_head: CommitId("recorded".into()),
        checked_out_head: Some(CommitId("head".into())),
        status: SubmoduleStatus::HeadMismatch,
    }]));
    repo_state.set_diff_target(Some(target.clone()));
    repo_state.diff_state.submodule_summary = Loadable::Ready(Arc::new(summary()));
    repo_state.diff_state.inline_submodule_diff = Some(crate::model::InlineSubmoduleDiffState {
        origin: crate::model::ForeignDiffOrigin::Submodule,
        submodule_repo_path: PathBuf::from("/tmp/repo/vendor/submodule"),
        parent_submodule_path: path.clone(),
        entries: Arc::clone(&entries),
        selected_ix: 0,
        target: inline_target.clone(),
        rev: 7,
        diff_rev: 1,
        diff: Loadable::Ready(Arc::new(gitcomet_core::domain::Diff {
            target: inline_target,
            lines: Vec::new(),
        })),
        diff_file_rev: 1,
        diff_file: Loadable::NotLoaded,
        diff_file_image: Loadable::NotLoaded,
    });
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::SubmoduleSummaryLoaded {
            repo_id: RepoId(1),
            target,
            result: Ok(summary()),
        }),
    );

    assert!(
        effects.is_empty(),
        "two fixed commits cannot have changed, got {effects:?}"
    );
    let inline = state.repos[0]
        .diff_state
        .inline_submodule_diff
        .as_ref()
        .expect("the inline diff stays open");
    assert_eq!(inline.rev, 7, "the inline generation must not move");
    assert!(Arc::ptr_eq(&entries, &inline.entries));
}

/// Feedback logs are shared between snapshots until a message writes them, so
/// a dispatch that leaves them alone does not copy every repository's logs.
#[test]
fn dispatches_that_leave_feedback_alone_keep_it_shared() {
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
    let snapshot = state.clone();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenInlineSubmoduleDiff {
            origin: crate::model::ForeignDiffOrigin::Submodule,
            repo_id: RepoId(1),
            submodule_repo_path: PathBuf::from("/tmp/repo/vendor/submodule"),
            parent_submodule_path: PathBuf::from("vendor/submodule"),
            entries: Arc::from(Vec::new()),
            selected_ix: 0,
        },
    );

    assert!(Arc::ptr_eq(
        &snapshot.repos[0].feedback,
        &state.repos[0].feedback
    ));
}
