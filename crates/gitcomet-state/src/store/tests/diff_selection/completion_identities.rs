use super::*;

#[test]
fn status_actions_close_only_matching_diffs_after_success() {
    use gitcomet_core::domain::DiffArea;
    for action in [
        RepoActionKind::StagePath,
        RepoActionKind::StagePaths,
        RepoActionKind::UnstagePath,
        RepoActionKind::UnstagePaths,
        RepoActionKind::DiscardWorktreeChangesPath,
        RepoActionKind::DiscardWorktreeChangesPaths,
    ] {
        let action_area = match action {
            RepoActionKind::UnstagePath | RepoActionKind::UnstagePaths => DiffArea::Staged,
            _ => DiffArea::Unstaged,
        };
        for selected_area in [DiffArea::Unstaged, DiffArea::Staged] {
            for paths in [
                vec![],
                vec![PathBuf::from("shown.rs")],
                vec![PathBuf::from("other.rs")],
            ] {
                // Single-path messages cannot express the all-paths case.
                if paths.is_empty()
                    && matches!(
                        action,
                        RepoActionKind::StagePath
                            | RepoActionKind::UnstagePath
                            | RepoActionKind::DiscardWorktreeChangesPath
                    )
                {
                    continue;
                }
                for (content_preview, edit_mode) in [(false, false), (true, false), (true, true)] {
                    for succeeds in [false, true] {
                        let target = DiffTarget::working_tree("shown.rs".into(), selected_area);
                        let mut state = status_action_state(target.clone());
                        state.repos[0].diff_state.content_preview = content_preview;
                        state.repos[0].diff_state.edit_mode = edit_mode;
                        let start = match action {
                            RepoActionKind::StagePath => Msg::StagePath {
                                repo_id: RepoId(1),
                                path: paths[0].clone(),
                            },
                            RepoActionKind::StagePaths => Msg::StagePaths {
                                repo_id: RepoId(1),
                                paths: paths.clone().into(),
                            },
                            RepoActionKind::UnstagePath => Msg::UnstagePath {
                                repo_id: RepoId(1),
                                path: paths[0].clone(),
                            },
                            RepoActionKind::UnstagePaths => Msg::UnstagePaths {
                                repo_id: RepoId(1),
                                paths: paths.clone().into(),
                            },
                            RepoActionKind::DiscardWorktreeChangesPath => {
                                Msg::DiscardWorktreeChangesPath {
                                    repo_id: RepoId(1),
                                    path: paths[0].clone(),
                                }
                            }
                            RepoActionKind::DiscardWorktreeChangesPaths => {
                                Msg::DiscardWorktreeChangesPaths {
                                    repo_id: RepoId(1),
                                    paths: paths.clone(),
                                }
                            }
                            _ => unreachable!(),
                        };
                        let effects = reduce(
                            &mut Default::default(),
                            &AtomicU64::new(3),
                            &mut state,
                            start,
                        );
                        assert_eq!(effects.len(), 1);
                        assert_eq!(
                            state.repos[0].diff_state.diff_target,
                            Some(target.clone()),
                            "starting an action must keep the diff"
                        );
                        finish_status_action(
                            &mut state,
                            RepoId(1),
                            action,
                            paths.clone(),
                            succeeds,
                        );
                        let should_close = succeeds
                            && !content_preview
                            && action_area == selected_area
                            && !(paths.is_empty()
                                && action == RepoActionKind::DiscardWorktreeChangesPaths)
                            && (paths.is_empty() || paths.contains(&PathBuf::from("shown.rs")));
                        let diff = &state.repos[0].diff_state;
                        assert_eq!(
                            diff.diff_target,
                            (!should_close).then_some(target),
                            "action={action:?} selected={selected_area:?} paths={paths:?} preview={content_preview} edit={edit_mode} succeeds={succeeds}"
                        );
                        assert_eq!(diff.content_preview, content_preview);
                        assert_eq!(diff.edit_mode, edit_mode);
                        assert_eq!(state.repos[0].local_actions_in_flight, 0);
                    }
                }
            }
        }
    }
}

#[test]
fn discarding_staged_files_closes_only_removed_additions() {
    use gitcomet_core::domain::DiffArea;
    for kind in [FileStatusKind::Added, FileStatusKind::Modified] {
        for has_worktree_changes in [false, true] {
            for paths in [vec![], vec![PathBuf::from("shown.rs")]] {
                let target = DiffTarget::working_tree("shown.rs".into(), DiffArea::Staged);
                let mut state = status_action_state(target.clone());
                state.repos[0].staged_status = Loadable::Ready(Arc::new(vec![FileStatus {
                    path: "shown.rs".into(),
                    kind,
                    conflict: None,
                }]));
                state.repos[0].worktree_status =
                    Loadable::Ready(Arc::new(if has_worktree_changes {
                        vec![FileStatus {
                            path: "shown.rs".into(),
                            kind: FileStatusKind::Modified,
                            conflict: None,
                        }]
                    } else {
                        vec![]
                    }));
                let removed =
                    !paths.is_empty() && kind == FileStatusKind::Added && !has_worktree_changes;
                finish_status_action(
                    &mut state,
                    RepoId(1),
                    RepoActionKind::DiscardWorktreeChangesPaths,
                    paths,
                    true,
                );
                assert_eq!(
                    state.repos[0].diff_state.diff_target,
                    (!removed).then_some(target)
                );
            }
        }
    }
}

#[test]
fn status_actions_preserve_historical_and_other_repository_views() {
    use gitcomet_core::domain::DiffArea;
    let working = DiffTarget::working_tree("shown.rs".into(), DiffArea::Unstaged);
    for target in [
        working.clone(),
        DiffTarget::commit(CommitId("head".into()), "shown.rs".into()),
        DiffTarget::commit_range(CommitId("base".into()), None, Some("shown.rs".into())),
    ] {
        let mut state = status_action_state(target.clone());
        let mut other = RepoState::new_opening(
            RepoId(2),
            RepoSpec {
                workdir: "/tmp/other".into(),
            },
        );
        other.set_diff_target(Some(working.clone()));
        state.repos.push(other);
        finish_status_action(
            &mut state,
            RepoId(2),
            RepoActionKind::StagePaths,
            vec![],
            true,
        );
        assert_eq!(state.repos[0].diff_state.diff_target, Some(target.clone()));
        assert_eq!(state.repos[1].diff_state.diff_target, None);
        if target != working {
            finish_status_action(
                &mut state,
                RepoId(1),
                RepoActionKind::StagePaths,
                vec![],
                true,
            );
            assert_eq!(state.repos[0].diff_state.diff_target, Some(target));
        }
    }
}

#[test]
fn unavailable_git_status_actions_preserve_the_diff() {
    use gitcomet_core::domain::DiffArea;
    for area in [DiffArea::Unstaged, DiffArea::Staged] {
        let target = DiffTarget::working_tree("shown.rs".into(), area);
        let mut state = status_action_state(target.clone());
        state.git_runtime.availability =
            gitcomet_core::process::GitExecutableAvailability::Unavailable {
                detail: "unavailable in test".into(),
            };
        let message = match area {
            DiffArea::Unstaged => Msg::StagePaths {
                repo_id: RepoId(1),
                paths: vec![].into(),
            },
            DiffArea::Staged => Msg::UnstagePaths {
                repo_id: RepoId(1),
                paths: vec![].into(),
            },
        };
        let effects = reduce(
            &mut Default::default(),
            &AtomicU64::new(3),
            &mut state,
            message,
        );
        assert!(effects.is_empty());
        assert_eq!(state.repos[0].diff_state.diff_target, Some(target));
        assert_eq!(state.repos[0].local_actions_in_flight, 0);
    }
}

#[test]
fn diff_loaded_err_records_diagnostic_when_target_matches() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    let target = DiffTarget::working_tree(
        PathBuf::from("src/lib.rs"),
        gitcomet_core::domain::DiffArea::Unstaged,
    );
    repo_state.diff_state.diff_target = Some(target.clone());
    repo_state.diff_state.diff = Loadable::Loading;
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));

    let error = Error::new(ErrorKind::Backend("diff failed".to_string()));
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::DiffLoaded {
            repo_id: RepoId(1),
            target,
            result: Err(error),
        }),
    );

    let repo_state = &state.repos[0];
    assert!(matches!(repo_state.diff_state.diff, Loadable::Error(_)));
    assert!(
        repo_state
            .feedback
            .diagnostics
            .iter()
            .any(|d| d.message.contains("diff failed"))
    );
}

#[test]
fn select_diff_bumps_diff_state_rev() {
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

    let before = state.repos[0].diff_state.diff_state_rev;

    let target = DiffTarget::working_tree(PathBuf::from("src/lib.rs"), DiffArea::Unstaged);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target,
        },
    );

    assert!(
        state.repos[0].diff_state.diff_state_rev > before,
        "diff_state_rev should bump after SelectDiff"
    );
}

#[test]
fn select_and_clear_diff_update_diff_target_rev_only_when_target_changes() {
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

    let first = DiffTarget::working_tree(PathBuf::from("src/lib.rs"), DiffArea::Unstaged);
    let second = DiffTarget::working_tree(PathBuf::from("src/main.rs"), DiffArea::Unstaged);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target: first.clone(),
        },
    );
    assert_eq!(state.repos[0].diff_state.diff_target_rev, 1);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target: first,
        },
    );
    assert_eq!(
        state.repos[0].diff_state.diff_target_rev, 1,
        "reselecting the same target should not invalidate queued selected-diff work"
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target: second,
        },
    );
    assert_eq!(state.repos[0].diff_state.diff_target_rev, 2);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ClearDiffSelection { repo_id: RepoId(1) },
    );
    assert_eq!(state.repos[0].diff_state.diff_target_rev, 3);
}

#[test]
fn clear_diff_selection_bumps_diff_state_rev() {
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

    // First select a diff
    let target = DiffTarget::working_tree(PathBuf::from("src/lib.rs"), DiffArea::Unstaged);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target,
        },
    );
    let before = state.repos[0].diff_state.diff_state_rev;

    // Now clear
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ClearDiffSelection { repo_id: RepoId(1) },
    );

    assert!(
        state.repos[0].diff_state.diff_state_rev > before,
        "diff_state_rev should bump after ClearDiffSelection"
    );
}

#[test]
fn select_diff_does_not_bump_unrelated_revs() {
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

    let ops_before = state.repos[0].ops_rev;
    let status_before = state.repos[0].status_rev;
    let log_before = state.repos[0].history_state.log_rev;

    let target = DiffTarget::working_tree(PathBuf::from("src/lib.rs"), DiffArea::Unstaged);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target,
        },
    );

    assert_eq!(state.repos[0].ops_rev, ops_before);
    assert_eq!(state.repos[0].status_rev, status_before);
    assert_eq!(state.repos[0].history_state.log_rev, log_before);
}

#[test]
fn select_and_clear_diff_are_noops_for_unknown_repo() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();

    let target = DiffTarget::working_tree(PathBuf::from("src/lib.rs"), DiffArea::Unstaged);
    let select = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(999),
            target,
        },
    );
    let clear = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ClearDiffSelection {
            repo_id: RepoId(999),
        },
    );

    assert!(select.is_empty());
    assert!(clear.is_empty());
    assert!(state.repos.is_empty());
}

#[test]
fn diff_loaded_ok_sets_ready_when_target_matches() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    let target = DiffTarget::working_tree(PathBuf::from("src/lib.rs"), DiffArea::Unstaged);
    repo_state.diff_state.diff_target = Some(target.clone());
    repo_state.diff_state.diff = Loadable::Loading;
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));

    let diff = gitcomet_core::domain::Diff {
        target: target.clone(),
        lines: vec![],
    };
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::DiffLoaded {
            repo_id: RepoId(1),
            target,
            result: Ok(diff),
        }),
    );

    let repo_state = &state.repos[0];
    assert!(matches!(repo_state.diff_state.diff, Loadable::Ready(_)));
    assert!(repo_state.feedback.diagnostics.is_empty());
}

#[test]
fn diff_loaded_identical_content_skips_rev_bumps_and_keeps_blame() {
    // A refresh that found no change must not churn the UI: window focus (and,
    // before the fix, every window drag) reloads the working-tree diff, and
    // bumping the revs would blank and re-run the expensive blame.
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let (mut state, target, blame) = state_with_loaded_diff_and_blame(unstaged_diff("let x = 1;"));
    let diff_rev = state.repos[0].diff_state.diff_rev;
    let diff_state_rev = state.repos[0].diff_state.diff_state_rev;
    let previous = match &state.repos[0].diff_state.diff {
        Loadable::Ready(diff) => Arc::clone(diff),
        other => panic!("expected a loaded diff, got {other:?}"),
    };

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::DiffLoaded {
            repo_id: RepoId(1),
            target: target.clone(),
            result: Ok(unstaged_diff("let x = 1;")),
        }),
    );

    let repo_state = &state.repos[0];
    assert_eq!(repo_state.diff_state.diff_rev, diff_rev);
    assert_eq!(repo_state.diff_state.diff_state_rev, diff_state_rev);
    assert!(
        matches!(&repo_state.diff_state.diff, Loadable::Ready(diff) if Arc::ptr_eq(diff, &previous)),
        "an unchanged reload must keep the existing Arc so identity fingerprints stay put"
    );
    assert!(
        matches!(&repo_state.diff_state.blame, Loadable::Ready(lines) if Arc::ptr_eq(lines, &blame)),
        "blame must survive a reload that found no change"
    );
}

#[test]
fn diff_loaded_changed_content_bumps_revs_and_invalidates_blame() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let (mut state, target, blame) = state_with_loaded_diff_and_blame(unstaged_diff("let x = 1;"));
    let diff_rev = state.repos[0].diff_state.diff_rev;
    let diff_state_rev = state.repos[0].diff_state.diff_state_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::DiffLoaded {
            repo_id: RepoId(1),
            target,
            result: Ok(unstaged_diff("let x = 2;")),
        }),
    );

    let repo_state = &state.repos[0];
    assert_eq!(repo_state.diff_state.diff_rev, diff_rev.wrapping_add(1));
    assert_eq!(
        repo_state.diff_state.diff_state_rev,
        diff_state_rev.wrapping_add(1)
    );
    assert!(
        matches!(repo_state.diff_state.blame, Loadable::NotLoaded),
        "blame is derived from the diff content, so changed content invalidates it"
    );
    // The target is preserved so the view reloads the same file's blame, and the
    // outgoing annotations stay painted meanwhile.
    assert_eq!(
        repo_state.diff_state.blame_path.as_deref(),
        Some(std::path::Path::new("src/lib.rs"))
    );
    assert!(
        repo_state
            .diff_state
            .retained_blame_while_loading
            .as_ref()
            .is_some_and(|held| Arc::ptr_eq(held, &blame))
    );
}

#[test]
fn diff_loaded_error_after_ready_still_bumps_revs() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let (mut state, target, _) = state_with_loaded_diff_and_blame(unstaged_diff("let x = 1;"));
    let diff_state_rev = state.repos[0].diff_state.diff_state_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::DiffLoaded {
            repo_id: RepoId(1),
            target,
            result: Err(Error::new(ErrorKind::Backend("boom".to_string()))),
        }),
    );

    let repo_state = &state.repos[0];
    assert!(matches!(repo_state.diff_state.diff, Loadable::Error(_)));
    assert_eq!(
        repo_state.diff_state.diff_state_rev,
        diff_state_rev.wrapping_add(1)
    );
    assert!(!repo_state.feedback.diagnostics.is_empty());
}

#[test]
fn diff_file_loaded_identical_content_skips_rev_bumps_and_keeps_blame() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let (mut state, target, blame) = state_with_loaded_file_text_and_blame(file_diff_text("a"));
    let diff_file_rev = state.repos[0].diff_state.diff_file_rev;
    let diff_state_rev = state.repos[0].diff_state.diff_state_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::DiffFileLoaded {
            repo_id: RepoId(1),
            target,
            result: Ok(Some(file_diff_text("a"))),
        }),
    );

    let repo_state = &state.repos[0];
    assert_eq!(repo_state.diff_state.diff_file_rev, diff_file_rev);
    assert_eq!(repo_state.diff_state.diff_state_rev, diff_state_rev);
    assert!(
        matches!(&repo_state.diff_state.blame, Loadable::Ready(lines) if Arc::ptr_eq(lines, &blame))
    );
}

#[test]
fn diff_file_loaded_changed_content_bumps_revs_and_invalidates_blame() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let (mut state, target, _) = state_with_loaded_file_text_and_blame(file_diff_text("a"));
    let diff_file_rev = state.repos[0].diff_state.diff_file_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::DiffFileLoaded {
            repo_id: RepoId(1),
            target,
            result: Ok(Some(file_diff_text("b"))),
        }),
    );

    let repo_state = &state.repos[0];
    assert_eq!(
        repo_state.diff_state.diff_file_rev,
        diff_file_rev.wrapping_add(1)
    );
    assert!(matches!(repo_state.diff_state.blame, Loadable::NotLoaded));
}

#[test]
fn diff_file_loaded_and_image_loaded_cover_success_and_error_paths() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    let target = DiffTarget::working_tree(PathBuf::from("icon.svg"), DiffArea::Unstaged);
    repo_state.diff_state.diff_target = Some(target.clone());
    repo_state.diff_state.diff_file = Loadable::Loading;
    repo_state.diff_state.diff_file_image = Loadable::Loading;
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::DiffFileLoaded {
            repo_id: RepoId(1),
            target: target.clone(),
            result: Ok(Some(gitcomet_core::domain::FileDiffText::new(
                PathBuf::from("icon.svg"),
                Some("old".to_string()),
                Some("new".to_string()),
            ))),
        }),
    );
    assert!(matches!(
        state.repos[0].diff_state.diff_file,
        Loadable::Ready(_)
    ));

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::DiffFileImageLoaded {
            repo_id: RepoId(1),
            target: target.clone(),
            result: Ok(Some(gitcomet_core::domain::FileDiffImage {
                path: PathBuf::from("icon.svg"),
                old: Some(vec![0x01]),
                new: Some(vec![0x02]),
                ..Default::default()
            })),
        }),
    );
    assert!(matches!(
        state.repos[0].diff_state.diff_file_image,
        Loadable::Ready(_)
    ));

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::DiffFileLoaded {
            repo_id: RepoId(1),
            target: target.clone(),
            result: Err(Error::new(ErrorKind::Backend(
                "text side-by-side failed".to_string(),
            ))),
        }),
    );
    assert!(matches!(
        state.repos[0].diff_state.diff_file,
        Loadable::Error(_)
    ));

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::DiffFileImageLoaded {
            repo_id: RepoId(1),
            target,
            result: Err(Error::new(ErrorKind::Backend(
                "image preview failed".to_string(),
            ))),
        }),
    );
    assert!(matches!(
        state.repos[0].diff_state.diff_file_image,
        Loadable::Error(_)
    ));
    assert!(
        state.repos[0]
            .feedback
            .diagnostics
            .iter()
            .any(|d| d.message.contains("text side-by-side failed"))
    );
    assert!(
        state.repos[0]
            .feedback
            .diagnostics
            .iter()
            .any(|d| d.message.contains("image preview failed"))
    );
}

#[test]
fn diff_results_are_ignored_for_non_matching_target() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    let selected = DiffTarget::working_tree(PathBuf::from("selected.txt"), DiffArea::Unstaged);
    let other = DiffTarget::working_tree(PathBuf::from("other.txt"), DiffArea::Unstaged);
    repo_state.diff_state.diff_target = Some(selected.clone());
    repo_state.diff_state.diff = Loadable::Loading;
    repo_state.diff_state.diff_file = Loadable::Loading;
    repo_state.diff_state.diff_file_image = Loadable::Loading;
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::DiffLoaded {
            repo_id: RepoId(1),
            target: other.clone(),
            result: Ok(gitcomet_core::domain::Diff {
                target: other.clone(),
                lines: vec![],
            }),
        }),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::DiffFileLoaded {
            repo_id: RepoId(1),
            target: other.clone(),
            result: Ok(None),
        }),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::DiffFileImageLoaded {
            repo_id: RepoId(1),
            target: other,
            result: Ok(None),
        }),
    );

    let repo_state = &state.repos[0];
    assert!(repo_state.diff_state.diff.is_loading());
    assert!(repo_state.diff_state.diff_file.is_loading());
    assert!(repo_state.diff_state.diff_file_image.is_loading());
    assert_eq!(repo_state.diff_state.diff_target, Some(selected));
}

#[test]
fn diff_file_image_loaded_drops_old_side_when_content_preview() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    let path = PathBuf::from("img.png");
    let target = DiffTarget::working_tree(path.clone(), DiffArea::Unstaged);
    repo_state.diff_state.content_preview = true;
    repo_state.diff_state.diff_target = Some(target.clone());
    repo_state.diff_state.diff_file_image = Loadable::Loading;
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));

    let image_data = vec![0xffu8, 0xd8, 0xff];
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::DiffFileImageLoaded {
            repo_id: RepoId(1),
            target,
            result: Ok(Some(FileDiffImage {
                path: path.clone(),
                old: Some(image_data.clone()),
                new: Some(image_data),
                ..Default::default()
            })),
        }),
    );

    let repo_state = state.repos.first().expect("repo state to exist");
    assert!(matches!(
        repo_state.diff_state.diff_file_image,
        Loadable::Ready(_)
    ));
    if let Loadable::Ready(Some(image)) = &repo_state.diff_state.diff_file_image {
        assert_eq!(image.path, path);
        assert!(image.old.is_none());
        assert!(image.new.is_some());
    }
}
