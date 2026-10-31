use super::*;

#[test]
fn worktree_commands_reload_worktrees_on_success() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();

    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::AddWorktree {
                path: PathBuf::from("/tmp/worktree"),
                checkout: gitcomet_core::services::WorktreeCheckout::NewBranch {
                    name: "wt".to_string(),
                    start: None,
                },
                open_after: false,
            },
            result: Ok(CommandOutput::empty_success(
                "git worktree add /tmp/worktree",
            )),
        }),
    );

    assert!(state.repos[0].worktrees.is_loading());
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadWorktrees { repo_id: id } if *id == repo_id))
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::WorktreesLoaded {
            repo_id,
            read: crate::msg::WorktreeListRead::explicit(),
            result: Ok(Arc::new(Vec::new())),
        }),
    );
    state.repos[0].set_worktrees(Loadable::NotLoaded);
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::RemoveWorktree {
                path: PathBuf::from("/tmp/worktree"),
                delete_branch: None,
            },
            result: Ok(CommandOutput::empty_success(
                "git worktree remove /tmp/worktree",
            )),
        }),
    );

    assert!(state.repos[0].worktrees.is_loading());
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadWorktrees { repo_id: id } if *id == repo_id))
    );
}

#[test]
fn worktree_commands_keep_sidebar_rows_until_refresh_finishes() {
    let repo_id = RepoId(1);
    let removed_path = PathBuf::from("/tmp/worktree");
    let existing = vec![
        gitcomet_core::domain::Worktree {
            bare: false,
            path: PathBuf::from("/tmp/repo"),
            head: Some(CommitId("a".into())),
            branch: Some("main".into()),
            detached: false,
        },
        gitcomet_core::domain::Worktree {
            bare: false,
            path: removed_path.clone(),
            head: Some(CommitId("b".into())),
            branch: Some("feature".into()),
            detached: false,
        },
    ];
    for command in [
        RepoCommandKind::AddWorktree {
            path: PathBuf::from("/tmp/new-worktree"),
            checkout: gitcomet_core::services::WorktreeCheckout::NewBranch {
                name: "wt".to_string(),
                start: None,
            },
            open_after: false,
        },
        RepoCommandKind::RemoveWorktree {
            path: removed_path.clone(),
            delete_branch: None,
        },
        RepoCommandKind::ForceRemoveWorktree {
            path: removed_path,
            delete_branch: None,
        },
    ] {
        let mut repos = RepositoryRuntimes::default();
        let id_alloc = AtomicU64::new(1);
        let mut state = AppState::test_default();
        let mut repo = RepoState::new_opening(
            repo_id,
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        );
        repo.open = Loadable::Ready(());
        repo.set_worktrees(Loadable::Ready(existing.clone()));
        let worktrees_rev = repo.worktrees_rev;
        state.repos.push(repo);
        state.active_repo = Some(repo_id);

        let mut refreshed = existing.clone();
        if let RepoCommandKind::AddWorktree { path, .. } = &command {
            let mut added = existing[1].clone();
            added.path = path.clone();
            refreshed.push(added);
        } else {
            refreshed.pop();
        }
        let effects = reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command,
                result: Ok(CommandOutput::empty_success("git worktree")),
            }),
        );

        assert_eq!(
            state.repos[0].worktrees,
            Loadable::Ready(Arc::new(existing.clone())),
            "replacing visible worktrees with a loading row collapses the sidebar scroll range"
        );
        assert_eq!(state.repos[0].worktrees_rev, worktrees_rev);
        assert!(effects.iter().any(
            |effect| matches!(effect, Effect::LoadWorktrees { repo_id: id } if *id == repo_id)
        ));

        assert!(
            state.repos[0]
                .loads_in_flight
                .is_in_flight(crate::model::LoadKind::Worktrees),
            "a worktree command must register the list load it starts"
        );
        let repeated = reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::LoadWorktrees { repo_id },
        );
        assert!(
            repeated.is_empty(),
            "a concurrent list request must coalesce"
        );

        let replay = reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::WorktreesLoaded {
                repo_id,
                read: crate::msg::WorktreeListRead::explicit(),
                result: Ok(Arc::new(refreshed.clone())),
            }),
        );
        assert!(matches!(replay.as_slice(), [Effect::LoadWorktrees { .. }]));
        assert_eq!(
            state.repos[0].worktrees,
            Loadable::Ready(Arc::new(refreshed))
        );
    }
}

#[test]
fn worktree_command_coalesces_with_an_existing_list_load() {
    let repo_id = RepoId(1);
    let mut repos = RepositoryRuntimes::default();
    let ids = AtomicU64::new(2);
    let mut repo = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: "/repo".into(),
        },
    );
    repo.set_open(Loadable::Ready(()));
    let mut state = AppState {
        repos: vec![repo],
        ..AppState::test_default()
    };
    assert_eq!(
        reduce(&mut repos, &ids, &mut state, Msg::LoadWorktrees { repo_id }).len(),
        1
    );
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::AddWorktree {
                path: "/linked".into(),
                checkout: gitcomet_core::services::WorktreeCheckout::NewBranch {
                    name: "wt".to_string(),
                    start: None,
                },
                open_after: false,
            },
            result: Ok(CommandOutput::empty_success("git worktree add")),
        }),
    );
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadWorktrees { .. })),
        "the command must queue a repeat, not occupy the list lane twice"
    );
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::WorktreesLoaded {
            repo_id,
            read: crate::msg::WorktreeListRead::explicit(),
            result: Ok(Arc::new(Vec::new())),
        }),
    );
    assert!(matches!(effects.as_slice(), [Effect::LoadWorktrees { .. }]));
    assert!(
        state.repos[0]
            .loads_in_flight
            .is_in_flight(crate::model::LoadKind::Worktrees)
    );
}

#[test]
fn worktree_remove_closes_tab_for_removed_worktree() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();

    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.repos.push(RepoState::new_opening(
        RepoId(2),
        RepoSpec {
            workdir: PathBuf::from("/tmp/worktree"),
        },
    ));
    state.active_repo = Some(RepoId(2));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id: RepoId(1),
            command: RepoCommandKind::RemoveWorktree {
                path: PathBuf::from("/tmp/worktree"),
                delete_branch: None,
            },
            result: Ok(CommandOutput::empty_success(
                "git worktree remove /tmp/worktree",
            )),
        }),
    );

    assert_eq!(state.repos.len(), 1);
    assert_eq!(state.repos[0].id, RepoId(1));
    assert_eq!(state.active_repo, Some(RepoId(1)));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadWorktrees { repo_id } if *repo_id == RepoId(1)))
    );
}

#[test]
fn discard_worktree_changes_path_emits_effect() {
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
        Msg::DiscardWorktreeChangesPath {
            repo_id: RepoId(1),
            path: PathBuf::from("a.txt"),
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::DiscardWorktreeChangesPath {
            repo_id: RepoId(1),
            path: _
        }]
    ));
}

#[test]
fn apply_worktree_patch_command_finished_reloads_png_diff_preview() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    let mut repo_state = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    let target = DiffTarget::working_tree(PathBuf::from("image.png"), DiffArea::Unstaged);
    repo_state.diff_state.diff_target = Some(target.clone());
    repo_state.diff_state.diff = Loadable::NotLoaded;
    repo_state.diff_state.diff_file = Loadable::NotLoaded;
    repo_state.diff_state.diff_file_image = Loadable::NotLoaded;
    state.repos.push(repo_state);
    state.active_repo = Some(repo_id);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::ApplyWorktreePatch { reverse: true },
            result: Ok(CommandOutput::empty_success("git apply -R")),
        }),
    );

    let repo_state = state.repos.first().expect("repo");
    assert!(repo_state.diff_state.diff.is_loading());
    assert!(matches!(
        repo_state.diff_state.diff_file,
        Loadable::NotLoaded
    ));
    assert!(repo_state.diff_state.diff_file_image.is_loading());
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadDiff {
            repo_id: RepoId(1),
            target: diff_target
        } if diff_target == &target
    )));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadDiffFileImage {
            repo_id: RepoId(1),
            target: diff_target
        } if diff_target == &target
    )));
    assert!(
        effects
            .iter()
            .all(|effect| !matches!(effect, Effect::LoadDiffFile { .. })),
        "png reload should request image preview only"
    );
}

fn finish_add_worktree(
    repos: &mut RepositoryRuntimes,
    id_alloc: &AtomicU64,
    state: &mut AppState,
    path: PathBuf,
    open_after: bool,
    result: std::result::Result<CommandOutput, gitcomet_core::error::Error>,
) -> Vec<Effect> {
    reduce(
        repos,
        id_alloc,
        state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id: RepoId(1),
            command: RepoCommandKind::AddWorktree {
                path,
                checkout: gitcomet_core::services::WorktreeCheckout::Existing {
                    reference: "feature".to_string(),
                },
                open_after,
            },
            result,
        }),
    )
}

/// A repo at `<tmp>/repo` beside an existing `<tmp>/wt` folder.
fn state_beside_new_worktree() -> (tempfile::TempDir, AppState, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let workdir = dir.path().join("repo");
    let worktree = dir.path().join("wt");
    std::fs::create_dir_all(&workdir).expect("workdir");
    std::fs::create_dir_all(&worktree).expect("worktree");
    let mut state = AppState::test_default();
    // The repo's own canonicaliser, as repositories are opened: on Windows it
    // drops the `\\?\` prefix `std::fs::canonicalize` adds.
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: gitcomet_core::path_utils::canonicalize_or_original(workdir),
        },
    ));
    state.active_repo = Some(RepoId(1));
    let worktree = gitcomet_core::path_utils::canonicalize_or_original(worktree);
    (dir, state, worktree)
}

#[test]
fn added_worktree_opens_in_a_tab_when_asked() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let (_dir, mut state, worktree) = state_beside_new_worktree();

    // Git ran in the workdir, so "../wt" is the sibling folder, not one beside
    // the process's own working directory.
    let effects = finish_add_worktree(
        &mut repos,
        &id_alloc,
        &mut state,
        PathBuf::from("../wt"),
        true,
        Ok(CommandOutput::empty_success(
            "git worktree add ../wt feature",
        )),
    );

    let opened = state
        .repos
        .iter()
        .find(|repo| repo.spec.workdir == worktree)
        .expect("the new worktree is opened as a tab");
    assert_eq!(state.active_repo, Some(opened.id));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::OpenRepo { repo_id, path } if *repo_id == opened.id && *path == worktree
    )));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadWorktrees { repo_id } if *repo_id == RepoId(1))),
        "the origin still refreshes its worktree list"
    );
}

#[test]
fn added_worktree_stays_closed_unless_asked() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let (_dir, mut state, worktree) = state_beside_new_worktree();

    let effects = finish_add_worktree(
        &mut repos,
        &id_alloc,
        &mut state,
        worktree,
        false,
        Ok(CommandOutput::empty_success("git worktree add wt feature")),
    );

    assert_eq!(state.repos.len(), 1);
    assert_eq!(state.active_repo, Some(RepoId(1)));
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::OpenRepo { .. }))
    );
}

#[test]
fn failed_worktree_add_opens_nothing() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let (_dir, mut state, worktree) = state_beside_new_worktree();

    let effects = finish_add_worktree(
        &mut repos,
        &id_alloc,
        &mut state,
        worktree,
        true,
        Err(gitcomet_core::error::Error::new(
            gitcomet_core::error::ErrorKind::Backend("already exists".into()),
        )),
    );

    assert_eq!(state.repos.len(), 1);
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::OpenRepo { .. }))
    );
}

#[test]
fn added_worktree_does_not_open_once_its_origin_tab_is_gone() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();

    // "../wt" was relative to the closed tab's workdir; nothing can resolve it now.
    let effects = finish_add_worktree(
        &mut repos,
        &id_alloc,
        &mut state,
        PathBuf::from("../wt"),
        true,
        Ok(CommandOutput::empty_success(
            "git worktree add ../wt feature",
        )),
    );

    assert!(state.repos.is_empty());
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::OpenRepo { .. }))
    );
}

fn state_with_origin_repo() -> AppState {
    let mut state = AppState::test_default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));
    state
}

fn removal_blocked_by_submodules(path: &str) -> Error {
    Error::new(ErrorKind::Git(gitcomet_core::error::GitFailure::new(
        format!("git worktree remove {path}"),
        gitcomet_core::error::GitFailureId::WorktreeRemovalBlocked(
            gitcomet_core::error::WorktreeRemovalBlocker::Submodules,
        ),
        Some(128),
        Vec::new(),
        b"fatal: working trees containing submodules cannot be moved or removed".to_vec(),
        None,
    )))
}

fn finish_removal(
    state: &mut AppState,
    command: RepoCommandKind,
    result: std::result::Result<CommandOutput, Error>,
) -> Vec<Effect> {
    let mut repos: RepositoryRuntimes = Default::default();
    reduce(
        &mut repos,
        &AtomicU64::new(1),
        state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id: RepoId(1),
            command,
            result,
        }),
    )
}

/// The prompt keeps the removal as data: a path ending in a space is still
/// the one forced, and the branch ticked in the first dialog still goes.
#[test]
fn refused_removal_offers_force_for_the_exact_worktree_and_branch() {
    let mut state = state_with_origin_repo();
    let path = "/tmp/feature ";

    finish_removal(
        &mut state,
        RepoCommandKind::RemoveWorktree {
            path: PathBuf::from(path),
            delete_branch: Some("feat/x".to_string()),
        },
        Err(removal_blocked_by_submodules(path)),
    );

    assert_eq!(
        state.force_remove_worktree_prompt,
        Some(crate::model::ForceRemoveWorktreePrompt {
            generation: 1,
            repo_id: RepoId(1),
            path: PathBuf::from(path),
            delete_branch: Some("feat/x".to_string()),
            blocker: gitcomet_core::error::WorktreeRemovalBlocker::Submodules,
        })
    );
    let entry = state.repos[0].feedback.command_log.last().expect("logged");
    assert!(
        entry.handled_by_prompt,
        "the prompt replaces the error toast"
    );
    assert_eq!(state.repos[0].feedback.last_error, None);
}

#[test]
fn other_removal_failures_offer_no_force() {
    let mut state = state_with_origin_repo();

    finish_removal(
        &mut state,
        RepoCommandKind::RemoveWorktree {
            path: PathBuf::from("/tmp/feature"),
            delete_branch: Some("feat/x".to_string()),
        },
        Err(Error::new(ErrorKind::Backend("not a working tree".into()))),
    );

    assert_eq!(state.force_remove_worktree_prompt, None);
    assert!(
        !state.repos[0]
            .feedback
            .command_log
            .last()
            .unwrap()
            .handled_by_prompt
    );
}

#[test]
fn forcing_answers_the_prompt_and_keeps_the_branch_request() {
    let mut state = state_with_origin_repo();
    finish_removal(
        &mut state,
        RepoCommandKind::RemoveWorktree {
            path: PathBuf::from("/tmp/feature"),
            delete_branch: Some("feat/x".to_string()),
        },
        Err(removal_blocked_by_submodules("/tmp/feature")),
    );

    let mut repos: RepositoryRuntimes = Default::default();
    let effects = reduce(
        &mut repos,
        &AtomicU64::new(1),
        &mut state,
        Msg::ForceRemoveWorktree {
            repo_id: RepoId(1),
            path: PathBuf::from("/tmp/feature"),
            delete_branch: Some("feat/x".to_string()),
        },
    );

    assert_eq!(state.force_remove_worktree_prompt, None);
    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::ForceRemoveWorktree {
            repo_id: RepoId(1),
            path,
            delete_branch: Some(branch),
        })] if path == &PathBuf::from("/tmp/feature") && branch == "feat/x"
    ));
}

#[test]
fn dismissing_clears_only_the_prompt_it_answers() {
    let mut state = state_with_origin_repo();
    finish_removal(
        &mut state,
        RepoCommandKind::RemoveWorktree {
            path: PathBuf::from("/tmp/feature"),
            delete_branch: None,
        },
        Err(removal_blocked_by_submodules("/tmp/feature")),
    );
    let shown = state.force_remove_worktree_prompt.clone().expect("prompt");
    let mut repos: RepositoryRuntimes = Default::default();

    let stale = crate::model::ForceRemoveWorktreePrompt {
        path: PathBuf::from("/tmp/other"),
        ..shown.clone()
    };
    reduce(
        &mut repos,
        &AtomicU64::new(1),
        &mut state,
        Msg::DismissForceRemoveWorktreePrompt { prompt: stale },
    );
    assert_eq!(state.force_remove_worktree_prompt.as_ref(), Some(&shown));

    reduce(
        &mut repos,
        &AtomicU64::new(1),
        &mut state,
        Msg::DismissForceRemoveWorktreePrompt { prompt: shown },
    );
    assert_eq!(state.force_remove_worktree_prompt, None);
}

#[test]
fn removal_deletes_the_requested_branch_only_once_the_worktree_is_gone() {
    let deletes_branch = |effects: &[Effect]| {
        effects.iter().any(|effect| {
            matches!(effect, Effect::DeleteBranch { repo_id: RepoId(1), name } if name == "feat/x")
        })
    };
    for force in [false, true] {
        let path = PathBuf::from("/tmp/feature");
        let delete_branch = Some("feat/x".to_string());
        let command = if force {
            RepoCommandKind::ForceRemoveWorktree {
                path,
                delete_branch,
            }
        } else {
            RepoCommandKind::RemoveWorktree {
                path,
                delete_branch,
            }
        };

        let mut state = state_with_origin_repo();
        let removed = finish_removal(
            &mut state,
            command.clone(),
            Ok(CommandOutput::empty_success(
                "git worktree remove /tmp/feature",
            )),
        );
        assert!(deletes_branch(&removed), "force={force}");

        let mut state = state_with_origin_repo();
        let refused = finish_removal(
            &mut state,
            command,
            Err(Error::new(ErrorKind::Backend("not a working tree".into()))),
        );
        assert!(!deletes_branch(&refused), "force={force}");
    }

    let mut state = state_with_origin_repo();
    let kept = finish_removal(
        &mut state,
        RepoCommandKind::RemoveWorktree {
            path: PathBuf::from("/tmp/feature"),
            delete_branch: None,
        },
        Ok(CommandOutput::empty_success(
            "git worktree remove /tmp/feature",
        )),
    );
    assert!(!deletes_branch(&kept), "no branch was asked for");
}

#[test]
fn closing_the_repo_drops_its_force_prompt() {
    let mut state = state_with_origin_repo();
    finish_removal(
        &mut state,
        RepoCommandKind::RemoveWorktree {
            path: PathBuf::from("/tmp/feature"),
            delete_branch: None,
        },
        Err(removal_blocked_by_submodules("/tmp/feature")),
    );
    assert!(state.force_remove_worktree_prompt.is_some());

    let mut repos: RepositoryRuntimes = Default::default();
    reduce(
        &mut repos,
        &AtomicU64::new(1),
        &mut state,
        Msg::CloseRepo { repo_id: RepoId(1) },
    );
    assert_eq!(state.force_remove_worktree_prompt, None);
}

/// The view opens the dialog when the prompt changes; a repeat of the same
/// refusal must still be a new prompt, and answering the old one leaves it.
#[test]
fn a_repeated_refusal_is_a_new_prompt() {
    let mut state = state_with_origin_repo();
    let refuse = |state: &mut AppState| {
        finish_removal(
            state,
            RepoCommandKind::RemoveWorktree {
                path: PathBuf::from("/tmp/feature"),
                delete_branch: None,
            },
            Err(removal_blocked_by_submodules("/tmp/feature")),
        );
        state.force_remove_worktree_prompt.clone().expect("prompt")
    };

    let first = refuse(&mut state);
    let second = refuse(&mut state);
    assert_ne!(first, second);

    let mut repos: RepositoryRuntimes = Default::default();
    reduce(
        &mut repos,
        &AtomicU64::new(1),
        &mut state,
        Msg::DismissForceRemoveWorktreePrompt { prompt: first },
    );
    assert_eq!(state.force_remove_worktree_prompt, Some(second));
}

#[test]
fn a_refusal_for_a_closed_repo_offers_no_prompt() {
    let mut state = AppState::test_default();

    finish_removal(
        &mut state,
        RepoCommandKind::RemoveWorktree {
            path: PathBuf::from("/tmp/feature"),
            delete_branch: None,
        },
        Err(removal_blocked_by_submodules("/tmp/feature")),
    );

    assert_eq!(state.force_remove_worktree_prompt, None);
}
