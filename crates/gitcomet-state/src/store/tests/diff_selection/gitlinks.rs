use super::*;

#[test]
fn select_diff_for_staged_deleted_head_gitlink_loads_submodule_summary() {
    let (_dir, mut repos, mut state, target) = staged_deleted_gitlink_fixture();
    let id_alloc = AtomicU64::new(2);

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
fn failed_head_change_rebuilds_selected_deleted_gitlink_before_reloading_diff() {
    let (_dir, mut repos, mut state, target) = staged_deleted_gitlink_fixture();
    let id_alloc = AtomicU64::new(2);
    let submodule_path = PathBuf::from("vendor/submodule");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target: target.clone(),
        },
    );
    assert!(state.repos[0].head_gitlink_paths.contains(&submodule_path));

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CheckoutBranch {
            repo_id: RepoId(1),
            name: "branch-that-will-fail".to_string(),
        },
    );
    assert!(
        !state.repos[0].head_gitlink_paths.contains(&submodule_path),
        "starting a HEAD-changing action should invalidate the old classification"
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
            repo_id: RepoId(1),
            action: RepoActionKind::CheckoutBranch,
            result: Err(Error::new(ErrorKind::Backend(
                "checkout failed".to_string(),
            ))),
        }),
    );

    assert!(state.repos[0].head_gitlink_paths.contains(&submodule_path));
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadSubmoduleSummary {
                repo_id: RepoId(1),
                target: reloaded,
            } if reloaded == &target
        )),
        "a failed HEAD change must reload the selected deletion as a submodule: {effects:?}"
    );
    assert!(
        !effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadDiff {
                repo_id: RepoId(1),
                target: reloaded,
            } if reloaded == &target
        )),
        "the deleted gitlink must not fall back to an ordinary patch diff: {effects:?}"
    );
}

#[test]
fn failed_revert_rebuilds_selected_deleted_gitlink() {
    let (_dir, mut repos, mut state, target) = staged_deleted_gitlink_fixture();
    let id_alloc = AtomicU64::new(2);
    let submodule_path = PathBuf::from("vendor/submodule");
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target: target.clone(),
        },
    );
    let commit_id =
        gitcomet_core::domain::CommitId("2222222222222222222222222222222222222222".into());
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RevertCommit {
            repo_id: RepoId(1),
            commit_id: commit_id.clone(),
            commit: true,
            mainline: None,
            summary: "revert me".to_string(),
        },
    );
    assert!(!state.repos[0].head_gitlink_paths.contains(&submodule_path));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id: RepoId(1),
            command: RepoCommandKind::Revert {
                commit_id,
                commit: true,
                mainline: None,
                summary: "revert me".to_string(),
            },
            result: Err(Error::new(ErrorKind::Backend(
                "revert: the index has staged changes".to_string(),
            ))),
        }),
    );

    assert!(
        state.repos[0].head_gitlink_paths.contains(&submodule_path),
        "a refused revert must restore the retained selection's classification"
    );
    assert!(
        !effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadDiff {
                repo_id: RepoId(1),
                target: reloaded,
            } if reloaded == &target
        )),
        "the deleted gitlink must not fall back to an ordinary patch diff: {effects:?}"
    );
}

#[test]
fn successful_head_change_rebuilds_selected_deleted_gitlink_before_reloading_diff() {
    let (_dir, mut repos, mut state, target) = staged_deleted_gitlink_fixture();
    let id_alloc = AtomicU64::new(2);
    let submodule_path = PathBuf::from("vendor/submodule");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target: target.clone(),
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CheckoutBranch {
            repo_id: RepoId(1),
            name: "branch-that-keeps-the-gitlink".to_string(),
        },
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
            repo_id: RepoId(1),
            action: RepoActionKind::CheckoutBranch,
            result: Ok(()),
        }),
    );

    assert!(state.repos[0].head_gitlink_paths.contains(&submodule_path));
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadSubmoduleSummary {
                repo_id: RepoId(1),
                target: reloaded,
            } if reloaded == &target
        )),
        "a successful HEAD change must reload the retained deletion as a submodule: {effects:?}"
    );
    assert!(
        !effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadDiff {
                repo_id: RepoId(1),
                target: reloaded,
            } if reloaded == &target
        )),
        "the retained deleted gitlink must not fall back to an ordinary patch diff: {effects:?}"
    );
}

#[test]
fn reload_repo_rebuilds_retained_deleted_gitlink_classification() {
    let (_dir, mut repos, mut state, target) = staged_deleted_gitlink_fixture();
    let id_alloc = AtomicU64::new(2);
    let submodule_path = PathBuf::from("vendor/submodule");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target,
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ReloadRepo { repo_id: RepoId(1) },
    );

    assert!(
        state.repos[0].head_gitlink_paths.contains(&submodule_path),
        "reload retains the diff target, so it must rebuild the target's HEAD classification"
    );
}

#[test]
fn activating_repo_rebuilds_retained_deleted_gitlink_classification() {
    let (_dir, mut repos, mut state, target) = staged_deleted_gitlink_fixture();
    let id_alloc = AtomicU64::new(2);
    let submodule_path = PathBuf::from("vendor/submodule");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target,
        },
    );
    state.repos[0].head_gitlink_paths.clear();
    state.active_repo = None;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: RepoId(1) },
    );

    assert!(state.repos[0].head_gitlink_paths.contains(&submodule_path));
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadSelectedDiff {
                repo_id: RepoId(1),
                load_patch_diff: false,
                load_submodule_summary: true,
                ..
            }
        )),
        "tab activation must plan the retained deletion as a submodule: {effects:?}"
    );
}

#[test]
fn external_head_moves_reclassify_retained_deletion_in_both_directions() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_test_repository(dir.path());
    let path = PathBuf::from("vendor/item");
    std::fs::create_dir_all(dir.path().join("vendor")).expect("vendor directory");
    std::fs::write(dir.path().join(&path), "ordinary file\n").expect("ordinary file");
    run_git(dir.path(), &["add", "vendor/item"]);
    run_git(dir.path(), &["commit", "-q", "-m", "ordinary file"]);
    run_git(dir.path(), &["branch", "ordinary-head"]);
    run_git(dir.path(), &["rm", "-q", "vendor/item"]);
    run_git(
        dir.path(),
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{},vendor/item", fake_gitlink_id(dir.path())),
        ],
    );
    run_git(dir.path(), &["commit", "-q", "-m", "gitlink"]);
    run_git(dir.path(), &["branch", "gitlink-head"]);
    run_git(dir.path(), &["rm", "-q", "--cached", "vendor/item"]);

    let mut repos: RepositoryRuntimes = Default::default();
    let backend = gitcomet_git_gix::GixBackend;
    repos.attach_repository_for_test(
        RepoId(1),
        backend
            .open(dir.path())
            .expect("open repository through backend"),
    );
    let mut state = AppState::test_default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: dir.path().to_path_buf(),
        },
    );
    repo_state.set_open(Loadable::Ready(()));
    repo_state.set_status(Loadable::Ready(Arc::new(RepoStatus {
        staged: std::sync::Arc::new(vec![FileStatus {
            path: path.clone(),
            kind: FileStatusKind::Deleted,
            conflict: None,
        }]),
        unstaged: std::sync::Arc::new(vec![]),
    })));
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));
    let target = gitcomet_core::domain::DiffTarget::working_tree(
        path.clone(),
        gitcomet_core::domain::DiffArea::Staged,
    );
    let id_alloc = AtomicU64::new(2);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target: target.clone(),
        },
    );
    assert!(state.repos[0].head_gitlink_paths.contains(&path));

    run_git(dir.path(), &["reset", "-q", "--soft", "ordinary-head"]);
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id: RepoId(1),
            change: crate::msg::RepoExternalChange::GitState,
        },
    );
    assert!(!state.repos[0].head_gitlink_paths.contains(&path));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadDiffPreviewTextFile {
            repo_id: RepoId(1),
            target: reloaded,
            side: gitcomet_core::domain::DiffPreviewTextSide::Old,
        } if reloaded == &target
    )));
    assert!(!effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadSubmoduleSummary {
            repo_id: RepoId(1),
            target: reloaded,
        } if reloaded == &target
    )));

    run_git(dir.path(), &["reset", "-q", "--soft", "gitlink-head"]);
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id: RepoId(1),
            change: crate::msg::RepoExternalChange::GitState,
        },
    );
    assert!(state.repos[0].head_gitlink_paths.contains(&path));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadSubmoduleSummary {
            repo_id: RepoId(1),
            target: reloaded,
        } if reloaded == &target
    )));
    assert!(!effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadDiff {
            repo_id: RepoId(1),
            target: reloaded,
        } if reloaded == &target
    )));
}

/// The HEAD-gitlink classification is filled where the diff target is set, not
/// where `SelectDiff` is dispatched, so every route back onto a staged
/// submodule deletion still renders it as a submodule summary rather than an
/// old-side text preview.
#[test]
fn global_nav_back_to_deleted_gitlink_keeps_submodule_classification() {
    let (dir, mut repos, mut state, target) = staged_deleted_gitlink_fixture();
    let id_alloc = AtomicU64::new(2);
    let submodule_path = PathBuf::from("vendor/submodule");

    // Add a second, ordinary staged file so there is somewhere else to navigate.
    let other = PathBuf::from("other.txt");
    std::fs::write(dir.path().join(&other), "hello\n").expect("write other");
    run_git(dir.path(), &["add", "other.txt"]);
    state.repos[0].set_status(Loadable::Ready(Arc::new(RepoStatus {
        staged: std::sync::Arc::new(vec![
            FileStatus {
                path: submodule_path.clone(),
                kind: FileStatusKind::Deleted,
                conflict: None,
            },
            FileStatus {
                path: other.clone(),
                kind: FileStatusKind::Added,
                conflict: None,
            },
        ]),
        unstaged: std::sync::Arc::new(vec![]),
    })));

    // 1. Select the staged submodule deletion -> classified, cached.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target: target.clone(),
        },
    );
    assert!(state.repos[0].head_gitlink_paths.contains(&submodule_path));

    // 2. Select an ordinary file.
    let other_target = gitcomet_core::domain::DiffTarget::working_tree(
        other.clone(),
        gitcomet_core::domain::DiffArea::Staged,
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target: other_target,
        },
    );

    // 3. Any external git-state change clears the cache and only reclassifies
    //    the *selected* path (the ordinary file).
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id: RepoId(1),
            change: crate::msg::RepoExternalChange::GitState,
        },
    );
    assert!(
        !state.repos[0].head_gitlink_paths.contains(&submodule_path),
        "precondition: the submodule's classification is gone"
    );

    // 4. Navigate back to the submodule.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::GlobalNavBack { repo_id: RepoId(1) },
    );

    assert_eq!(
        state.repos[0].diff_state.diff_target,
        Some(target.clone()),
        "nav-back must land on the submodule target"
    );
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadSelectedDiff {
                repo_id: RepoId(1),
                load_submodule_summary: true,
                ..
            }
        )),
        "nav-back to the staged submodule deletion must plan a submodule summary: {effects:?}"
    );
}
