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
        },
        RepoCommandKind::ForceRemoveWorktree { path: removed_path },
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
