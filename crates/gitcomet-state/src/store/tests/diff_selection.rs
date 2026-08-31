use super::*;
use gitcomet_core::domain::{
    CommitDetails, CommitFileChange, CommitId, FileConflictKind, FileDiffImage, FileSource,
    FileStatus, FileStatusKind, Submodule, SubmoduleDiffSummary, SubmoduleDiffSummaryMode,
    SubmoduleInnerChange, SubmoduleStatus,
};

#[test]
fn select_diff_sets_loading_and_emits_effect() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: PathBuf::from("src/lib.rs"),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target: target.clone(),
        },
    );

    let repo_state = state.repos.first().expect("repo state to exist");
    assert_eq!(repo_state.diff_state.diff_target, Some(target.clone()));
    assert!(repo_state.diff_state.diff.is_loading());
    assert!(repo_state.diff_state.diff_file.is_loading());
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadSelectedDiff {
            repo_id: RepoId(1),
            load_patch_diff: true,
            load_file_text: true,
            load_file_image: false,
            load_submodule_summary: false,
            preview_text_side: None,
        }]
    ));
}

#[test]
fn select_diff_for_image_sets_loading_and_emits_effect() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: PathBuf::from("img.png"),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target: target.clone(),
        },
    );

    let repo_state = state.repos.first().expect("repo state to exist");
    assert_eq!(repo_state.diff_state.diff_target, Some(target.clone()));
    assert!(repo_state.diff_state.diff.is_loading());
    assert!(matches!(
        repo_state.diff_state.diff_file,
        Loadable::NotLoaded
    ));
    assert!(repo_state.diff_state.diff_file_image.is_loading());
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadSelectedDiff {
            repo_id: RepoId(1),
            load_patch_diff: true,
            load_file_text: false,
            load_file_image: true,
            load_submodule_summary: false,
            preview_text_side: None,
        }]
    ));
}

#[test]
fn select_diff_for_ico_sets_loading_and_emits_effect() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: PathBuf::from("app.ico"),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target: target.clone(),
        },
    );

    let repo_state = state.repos.first().expect("repo state to exist");
    assert_eq!(repo_state.diff_state.diff_target, Some(target.clone()));
    assert!(repo_state.diff_state.diff.is_loading());
    assert!(matches!(
        repo_state.diff_state.diff_file,
        Loadable::NotLoaded
    ));
    assert!(repo_state.diff_state.diff_file_image.is_loading());
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadSelectedDiff {
            repo_id: RepoId(1),
            load_patch_diff: true,
            load_file_text: false,
            load_file_image: true,
            load_submodule_summary: false,
            preview_text_side: None,
        }]
    ));
}

#[test]
fn select_diff_for_svg_loads_image_and_text() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: PathBuf::from("icon.svg"),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target: target.clone(),
        },
    );

    let repo_state = state.repos.first().expect("repo state to exist");
    assert_eq!(repo_state.diff_state.diff_target, Some(target.clone()));
    assert!(repo_state.diff_state.diff.is_loading());
    assert!(repo_state.diff_state.diff_file.is_loading());
    assert!(repo_state.diff_state.diff_file_image.is_loading());
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadSelectedDiff {
            repo_id: RepoId(1),
            load_patch_diff: true,
            load_file_text: true,
            load_file_image: true,
            load_submodule_summary: false,
            preview_text_side: None,
        }]
    ));
}

#[test]
fn select_diff_for_untracked_file_skips_patch_diff_and_loads_file_preview() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: PathBuf::from("report.json"),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };
    repo_state.set_status(Loadable::Ready(Arc::new(RepoStatus {
        unstaged: vec![FileStatus {
            path: PathBuf::from("report.json"),
            kind: FileStatusKind::Untracked,
            conflict: None,
        }],
        staged: vec![],
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
    assert!(repo_state.diff_state.diff_preview_text_file.is_loading());
    assert!(matches!(
        repo_state.diff_state.diff_file_image,
        Loadable::NotLoaded
    ));
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadSelectedDiff {
            repo_id: RepoId(1),
            load_patch_diff: false,
            load_file_text: false,
            preview_text_side: Some(gitcomet_core::domain::DiffPreviewTextSide::New),
            load_file_image: false,
            load_submodule_summary: false,
        }]
    ));
}

#[test]
fn select_diff_for_deleted_file_replaced_by_directory_loads_deleted_preview() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(dir.path().join("report.json")).expect("replacement directory");

    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: dir.path().to_path_buf(),
        },
    );
    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: PathBuf::from("report.json"),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };
    repo_state.set_status(Loadable::Ready(Arc::new(RepoStatus {
        unstaged: vec![FileStatus {
            path: PathBuf::from("report.json"),
            kind: FileStatusKind::Deleted,
            conflict: None,
        }],
        staged: vec![],
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
    assert!(repo_state.diff_state.diff_preview_text_file.is_loading());
    assert!(matches!(
        repo_state.diff_state.submodule_summary,
        Loadable::NotLoaded
    ));
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadSelectedDiff {
            repo_id: RepoId(1),
            load_patch_diff: false,
            load_file_text: false,
            preview_text_side: Some(gitcomet_core::domain::DiffPreviewTextSide::Old),
            load_file_image: false,
            load_submodule_summary: false,
        }]
    ));
}

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

    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: dir.path().to_path_buf(),
        },
    );
    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: submodule_path.clone(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };
    repo_state.set_status(Loadable::Ready(Arc::new(RepoStatus {
        unstaged: vec![FileStatus {
            path: submodule_path,
            kind: FileStatusKind::Modified,
            conflict: None,
        }],
        staged: vec![],
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

fn staged_deleted_gitlink_fixture() -> (
    tempfile::TempDir,
    FxHashMap<RepoId, Arc<dyn GitRepository>>,
    AppState,
    gitcomet_core::domain::DiffTarget,
) {
    let dir = tempfile::tempdir().expect("tempdir");
    init_test_repository(dir.path());
    run_git(
        dir.path(),
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            "160000,1111111111111111111111111111111111111111,vendor/submodule",
        ],
    );
    run_git(dir.path(), &["commit", "-q", "-m", "add submodule gitlink"]);

    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let mut state = AppState::default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: dir.path().to_path_buf(),
        },
    );
    let submodule_path = PathBuf::from("vendor/submodule");
    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: submodule_path.clone(),
        area: gitcomet_core::domain::DiffArea::Staged,
    };
    repo_state.set_open(Loadable::Ready(()));
    repo_state.set_status(Loadable::Ready(Arc::new(RepoStatus {
        staged: vec![FileStatus {
            path: submodule_path,
            kind: FileStatusKind::Deleted,
            conflict: None,
        }],
        unstaged: vec![],
    })));
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));

    let backend = gitcomet_git_gix::GixBackend;
    repos.insert(
        RepoId(1),
        backend
            .open(dir.path())
            .expect("open repository through backend"),
    );

    (dir, repos, state, target)
}

fn init_test_repository(path: &Path) {
    run_git(path, &["init", "-q"]);
    run_git(path, &["config", "commit.gpgsign", "false"]);
    run_git(path, &["config", "user.name", "Test User"]);
    run_git(path, &["config", "user.email", "test@example.com"]);
}

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
            "160000,1111111111111111111111111111111111111111,vendor/item",
        ],
    );
    run_git(dir.path(), &["commit", "-q", "-m", "gitlink"]);
    run_git(dir.path(), &["branch", "gitlink-head"]);
    run_git(dir.path(), &["rm", "-q", "--cached", "vendor/item"]);

    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let backend = gitcomet_git_gix::GixBackend;
    repos.insert(
        RepoId(1),
        backend
            .open(dir.path())
            .expect("open repository through backend"),
    );
    let mut state = AppState::default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: dir.path().to_path_buf(),
        },
    );
    repo_state.set_open(Loadable::Ready(()));
    repo_state.set_status(Loadable::Ready(Arc::new(RepoStatus {
        staged: vec![FileStatus {
            path: path.clone(),
            kind: FileStatusKind::Deleted,
            conflict: None,
        }],
        unstaged: vec![],
    })));
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));
    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: path.clone(),
        area: gitcomet_core::domain::DiffArea::Staged,
    };
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

#[test]
fn select_diff_for_deleted_commit_file_skips_patch_diff_and_loads_file_preview() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    let commit_id = CommitId("deadbeef".into());
    let target = gitcomet_core::domain::DiffTarget::Commit {
        commit_id: commit_id.clone(),
        path: Some(PathBuf::from("report.json")),
    };
    repo_state.history_state.commit_details = Loadable::Ready(Arc::new(CommitDetails {
        id: commit_id,
        message: "remove report".to_string(),
        author_name: String::new(),
        author_email: String::new(),
        authored_at_unix: 0,
        committed_at: "2026-04-07T12:00:00Z".to_string(),
        committed_at_unix: 0,
        parent_ids: vec![],
        files: vec![CommitFileChange {
            path: PathBuf::from("report.json"),
            kind: FileStatusKind::Deleted,
            is_submodule: false,
            additions: None,
            deletions: None,
        }],
    }));
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
    assert!(repo_state.diff_state.diff_preview_text_file.is_loading());
    assert!(matches!(
        repo_state.diff_state.diff_file_image,
        Loadable::NotLoaded
    ));
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadSelectedDiff {
            repo_id: RepoId(1),
            load_patch_diff: false,
            load_file_text: false,
            preview_text_side: Some(gitcomet_core::domain::DiffPreviewTextSide::Old),
            load_file_image: false,
            load_submodule_summary: false,
        }]
    ));
}

#[test]
fn open_inline_submodule_diff_loads_patch_and_file_text_for_text_targets() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let target = gitcomet_core::domain::DiffTarget::CommitRange {
        from_commit_id: CommitId("aaaa".into()),
        to_commit_id: Some(CommitId("bbbb".into())),
        path: Some(PathBuf::from("src/lib.rs")),
    };
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
            }],
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
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: PathBuf::from("icons/logo.svg"),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };
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
            }],
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
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let target = gitcomet_core::domain::DiffTarget::CommitRange {
        from_commit_id: CommitId("aaaa".into()),
        to_commit_id: Some(CommitId("bbbb".into())),
        path: Some(PathBuf::from("src/lib.rs")),
    };
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
            }],
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
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let target = gitcomet_core::domain::DiffTarget::CommitRange {
        from_commit_id: CommitId("aaaa".into()),
        to_commit_id: Some(CommitId("bbbb".into())),
        path: Some(PathBuf::from("src/lib.rs")),
    };
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
            entries: vec![entry.clone()],
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
            entries: vec![entry],
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
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let parent_path = PathBuf::from("vendor/submodule");
    let parent_target = DiffTarget::WorkingTree {
        path: parent_path.clone(),
        area: DiffArea::Unstaged,
    };
    let inline_target = DiffTarget::WorkingTree {
        path: PathBuf::from("src/lib.rs"),
        area: DiffArea::Unstaged,
    };
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.set_status(Loadable::Ready(Arc::new(RepoStatus {
        unstaged: vec![FileStatus {
            path: parent_path.clone(),
            kind: FileStatusKind::Modified,
            conflict: None,
        }],
        staged: vec![],
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
        commit_id: None,
        parent_commit_id: None,
        checked_out_head: Some(CommitId("old-head".into())),
        ranges: vec![],
        live_staged: vec![],
        live_unstaged: vec![SubmoduleInnerChange {
            path: PathBuf::from("src/lib.rs"),
            kind: FileStatusKind::Modified,
            additions: Some(1),
            deletions: Some(1),
        }],
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
        }],
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
                commit_id: None,
                parent_commit_id: None,
                checked_out_head: Some(CommitId("new-head".into())),
                ranges: vec![],
                live_staged: vec![],
                live_unstaged: vec![
                    SubmoduleInnerChange {
                        path: PathBuf::from("README.md"),
                        kind: FileStatusKind::Modified,
                        additions: Some(2),
                        deletions: Some(0),
                    },
                    SubmoduleInnerChange {
                        path: PathBuf::from("src/lib.rs"),
                        kind: FileStatusKind::Modified,
                        additions: Some(4),
                        deletions: Some(1),
                    },
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
fn commit_details_loaded_replans_selected_deleted_commit_file_to_preview_text_file() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    let commit_id = CommitId("deadbeef".into());
    let target = gitcomet_core::domain::DiffTarget::Commit {
        commit_id: commit_id.clone(),
        path: Some(PathBuf::from("report.json")),
    };
    repo_state.set_selected_commit(Some(commit_id.clone()));
    repo_state.diff_state.diff_target = Some(target.clone());
    repo_state.diff_state.diff = Loadable::Loading;
    repo_state.diff_state.diff_file = Loadable::Loading;
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitDetailsLoaded {
            repo_id: RepoId(1),
            commit_id: commit_id.clone(),
            result: Ok(CommitDetails {
                id: commit_id,
                message: "remove report".to_string(),
                author_name: String::new(),
                author_email: String::new(),
                authored_at_unix: 0,
                committed_at: "2026-04-07T12:00:00Z".to_string(),
                committed_at_unix: 0,
                parent_ids: vec![],
                files: vec![CommitFileChange {
                    path: PathBuf::from("report.json"),
                    kind: FileStatusKind::Deleted,
                    is_submodule: false,
                    additions: None,
                    deletions: None,
                }],
            }),
        }),
    );

    let repo_state = state.repos.first().expect("repo state to exist");
    assert!(matches!(
        repo_state.history_state.commit_details,
        Loadable::Ready(_)
    ));
    assert!(matches!(repo_state.diff_state.diff, Loadable::NotLoaded));
    assert!(matches!(
        repo_state.diff_state.diff_file,
        Loadable::NotLoaded
    ));
    assert!(repo_state.diff_state.diff_preview_text_file.is_loading());
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadDiffPreviewTextFile {
            repo_id: RepoId(1),
            target: effect_target,
            side: gitcomet_core::domain::DiffPreviewTextSide::Old,
        }] if effect_target == &target
    ));

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::DiffFileLoaded {
            repo_id: RepoId(1),
            target,
            result: Ok(Some(gitcomet_core::domain::FileDiffText::new(
                PathBuf::from("report.json"),
                Some("old".to_string()),
                None,
            ))),
        }),
    );

    assert!(matches!(
        state.repos[0].diff_state.diff_file,
        Loadable::NotLoaded
    ));
}

#[test]
fn select_diff_for_conflicted_file_skips_patch_and_file_diff_loads() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: PathBuf::from("index.html"),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };
    repo_state.set_status(Loadable::Ready(Arc::new(RepoStatus {
        unstaged: vec![FileStatus {
            path: PathBuf::from("index.html"),
            kind: FileStatusKind::Conflicted,
            conflict: Some(FileConflictKind::BothModified),
        }],
        staged: vec![],
    })));
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target: target.clone(),
        },
    );

    let repo_state = state.repos.first().expect("repo state to exist");
    assert_eq!(repo_state.diff_state.diff_target, Some(target));
    assert!(matches!(repo_state.diff_state.diff, Loadable::NotLoaded));
    assert!(matches!(
        repo_state.diff_state.diff_file,
        Loadable::NotLoaded
    ));
    assert!(matches!(
        repo_state.diff_state.diff_file_image,
        Loadable::NotLoaded
    ));
    assert_eq!(
        repo_state.conflict_state.conflict_file_path.as_deref(),
        Some(std::path::Path::new("index.html"))
    );
    assert!(repo_state.conflict_state.conflict_file.is_loading());
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadSelectedConflictFile {
            repo_id: RepoId(1),
            mode: crate::model::ConflictFileLoadMode::CurrentOnly
        }]
    ));
}

#[test]
fn select_diff_for_conflicted_svg_prefers_conflict_loader_over_preview_effects() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: PathBuf::from("icon.svg"),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };
    repo_state.set_status(Loadable::Ready(Arc::new(RepoStatus {
        unstaged: vec![FileStatus {
            path: PathBuf::from("icon.svg"),
            kind: FileStatusKind::Conflicted,
            conflict: Some(FileConflictKind::BothModified),
        }],
        staged: vec![],
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
    assert!(matches!(
        repo_state.diff_state.diff_file_image,
        Loadable::NotLoaded
    ));
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadSelectedConflictFile {
            repo_id: RepoId(1),
            mode: crate::model::ConflictFileLoadMode::CurrentOnly
        }]
    ));
}

#[test]
fn select_diff_for_commit_without_path_only_loads_patch() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let target = gitcomet_core::domain::DiffTarget::Commit {
        commit_id: CommitId("deadbeef".into()),
        path: None,
    };

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target: target.clone(),
        },
    );

    let repo_state = state.repos.first().expect("repo state to exist");
    assert_eq!(repo_state.diff_state.diff_target, Some(target.clone()));
    assert!(repo_state.diff_state.diff.is_loading());
    assert!(matches!(
        repo_state.diff_state.diff_file,
        Loadable::NotLoaded
    ));
    assert!(matches!(
        repo_state.diff_state.diff_file_image,
        Loadable::NotLoaded
    ));
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadSelectedDiff {
            repo_id: RepoId(1),
            load_patch_diff: true,
            load_file_text: false,
            load_file_image: false,
            load_submodule_summary: false,
            preview_text_side: None,
        }]
    ));
}

#[test]
fn select_diff_for_commit_svg_path_loads_text_and_image_previews() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let target = gitcomet_core::domain::DiffTarget::Commit {
        commit_id: CommitId("deadbeef".into()),
        path: Some(PathBuf::from("diagram.svg")),
    };

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target: target.clone(),
        },
    );

    let repo_state = state.repos.first().expect("repo state to exist");
    assert_eq!(repo_state.diff_state.diff_target, Some(target.clone()));
    assert!(repo_state.diff_state.diff.is_loading());
    assert!(repo_state.diff_state.diff_file.is_loading());
    assert!(repo_state.diff_state.diff_file_image.is_loading());
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadSelectedDiff {
            repo_id: RepoId(1),
            load_patch_diff: true,
            load_file_text: true,
            load_file_image: true,
            load_submodule_summary: false,
            preview_text_side: None,
        }]
    ));
}

#[test]
fn stage_hunk_emits_effect() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
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
        Msg::StageHunk {
            repo_id: RepoId(1),
            patch: "diff --git a/a.txt b/a.txt\n".to_string(),
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::StageHunk {
            repo_id: RepoId(1),
            patch: _
        }]
    ));
}

#[test]
fn unstage_hunk_emits_effect() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
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
        Msg::UnstageHunk {
            repo_id: RepoId(1),
            patch: "diff --git a/a.txt b/a.txt\n".to_string(),
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::UnstageHunk {
            repo_id: RepoId(1),
            patch: _
        }]
    ));
}

#[test]
fn stage_hunk_command_finished_reloads_current_diff() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.diff_state.diff_target = Some(DiffTarget::WorkingTree {
        path: PathBuf::from("a.txt"),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    });
    repo_state.diff_state.diff = Loadable::NotLoaded;
    repo_state.diff_state.diff_file = Loadable::NotLoaded;
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id: RepoId(1),
            command: crate::msg::RepoCommandKind::StageHunk,
            result: Ok(CommandOutput::default()),
        }),
    );

    let repo_state = state.repos.iter().find(|r| r.id == RepoId(1)).unwrap();
    assert!(repo_state.diff_state.diff.is_loading());
    assert!(repo_state.diff_state.diff_file.is_loading());
    assert!(effects.iter().any(|e| {
        matches!(e, Effect::LoadDiff { repo_id: RepoId(1), target: DiffTarget::WorkingTree { path, area: gitcomet_core::domain::DiffArea::Unstaged } } if path == &PathBuf::from("a.txt"))
    }));
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::LoadDiffFile {
            repo_id: RepoId(1),
            target: _
        }
    )));
}

#[test]
fn stage_hunk_command_finished_keeps_loaded_diff_visible_while_reloading() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    let target = DiffTarget::WorkingTree {
        path: PathBuf::from("a.txt"),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };
    repo_state.diff_state.diff_target = Some(target.clone());
    repo_state.diff_state.diff = Loadable::Ready(Arc::new(gitcomet_core::domain::Diff {
        target: target.clone(),
        lines: vec![gitcomet_core::domain::DiffLine {
            kind: gitcomet_core::domain::DiffLineKind::Add,
            text: "+one".into(),
        }],
    }));
    repo_state.diff_state.diff_file =
        Loadable::Ready(Some(Arc::new(gitcomet_core::domain::FileDiffText::new(
            PathBuf::from("a.txt"),
            Some("one\n".to_string()),
            Some("two\n".to_string()),
        ))));
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id: RepoId(1),
            command: crate::msg::RepoCommandKind::StageHunk,
            result: Ok(CommandOutput::default()),
        }),
    );

    let repo_state = state.repos.iter().find(|r| r.id == RepoId(1)).unwrap();
    // Staging reloads the same target, so the pane keeps rendering what it has
    // until the fresh payload lands instead of flashing a loading placeholder.
    assert!(
        matches!(repo_state.diff_state.diff, Loadable::Ready(_)),
        "the loaded patch diff must survive a same-target reload"
    );
    assert!(
        matches!(repo_state.diff_state.diff_file, Loadable::Ready(_)),
        "the loaded file text must survive a same-target reload"
    );
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::LoadDiff {
            repo_id: RepoId(1),
            ..
        }
    )));
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::LoadDiffFile {
            repo_id: RepoId(1),
            ..
        }
    )));

    // Kept content is content from before the command: it describes the index as
    // it was, so a patch cut out of it no longer applies. The command itself is
    // already done by this point, so the flag is the only thing that says so.
    assert!(
        repo_state.diff_state.diff_reload_in_flight,
        "rows kept from before the reload must be flagged as a generation behind"
    );
    assert_eq!(
        repo_state.local_actions_in_flight, 0,
        "the command is finished here — the flag is what covers the rest of the window"
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::DiffLoaded {
            repo_id: RepoId(1),
            target: target.clone(),
            result: Ok(gitcomet_core::domain::Diff {
                target: target.clone(),
                lines: vec![gitcomet_core::domain::DiffLine {
                    kind: gitcomet_core::domain::DiffLineKind::Add,
                    text: "+two".into(),
                }],
            }),
        }),
    );

    let repo_state = state.repos.iter().find(|r| r.id == RepoId(1)).unwrap();
    assert!(
        !repo_state.diff_state.diff_reload_in_flight,
        "the reload landing must clear the flag"
    );
}

/// A target change blanks the diff outright, so there are no stale rows to guard
/// and the flag must not be left set by a reload that will never land.
#[test]
fn selecting_a_different_diff_clears_the_reload_in_flight_flag() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.diff_state.diff_reload_in_flight = true;
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target: DiffTarget::WorkingTree {
                path: PathBuf::from("b.txt"),
                area: gitcomet_core::domain::DiffArea::Unstaged,
            },
        },
    );

    let repo_state = state.repos.iter().find(|r| r.id == RepoId(1)).unwrap();
    assert!(!repo_state.diff_state.diff_reload_in_flight);
}

#[test]
fn clear_diff_selection_resets_diff_state() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::WorkingTree {
        path: PathBuf::from("src/lib.rs"),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    });
    repo_state.diff_state.diff = Loadable::Loading;
    repo_state.diff_state.diff_file = Loadable::Loading;
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ClearDiffSelection { repo_id: RepoId(1) },
    );

    let repo_state = state.repos.first().expect("repo state to exist");
    assert!(repo_state.diff_state.diff_target.is_none());
    assert!(matches!(repo_state.diff_state.diff, Loadable::NotLoaded));
    assert!(matches!(
        repo_state.diff_state.diff_file,
        Loadable::NotLoaded
    ));
    assert!(effects.is_empty());
}

#[test]
fn diff_loaded_err_records_diagnostic_when_target_matches() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    let target = DiffTarget::WorkingTree {
        path: PathBuf::from("src/lib.rs"),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };
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

// --- Revision counter regression tests ---

#[test]
fn select_diff_bumps_diff_state_rev() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let before = state.repos[0].diff_state.diff_state_rev;

    let target = DiffTarget::WorkingTree {
        path: PathBuf::from("src/lib.rs"),
        area: DiffArea::Unstaged,
    };
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
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let first = DiffTarget::WorkingTree {
        path: PathBuf::from("src/lib.rs"),
        area: DiffArea::Unstaged,
    };
    let second = DiffTarget::WorkingTree {
        path: PathBuf::from("src/main.rs"),
        area: DiffArea::Unstaged,
    };

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
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    // First select a diff
    let target = DiffTarget::WorkingTree {
        path: PathBuf::from("src/lib.rs"),
        area: DiffArea::Unstaged,
    };
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
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
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

    let target = DiffTarget::WorkingTree {
        path: PathBuf::from("src/lib.rs"),
        area: DiffArea::Unstaged,
    };
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
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let target = DiffTarget::WorkingTree {
        path: PathBuf::from("src/lib.rs"),
        area: DiffArea::Unstaged,
    };
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
fn apply_worktree_patch_emits_effect() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
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
            patch: "@@ -1 +1 @@\n-old\n+new\n".to_string(),
            reverse: false,
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::ApplyWorktreePatch {
            repo_id: RepoId(1),
            reverse: false,
            ..
        }]
    ));
}

#[test]
fn diff_loaded_ok_sets_ready_when_target_matches() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    let target = DiffTarget::WorkingTree {
        path: PathBuf::from("src/lib.rs"),
        area: DiffArea::Unstaged,
    };
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

/// A repo with a working-tree diff and blame both loaded, ready to be fed a
/// `DiffLoaded` / `DiffFileLoaded` result.
fn state_with_loaded_diff_and_blame(
    diff: gitcomet_core::domain::Diff,
) -> (
    AppState,
    DiffTarget,
    Arc<Vec<gitcomet_core::services::BlameLine>>,
) {
    let mut state = AppState::default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    let target = diff.target.clone();
    repo_state.diff_state.diff_target = Some(target.clone());
    repo_state.diff_state.diff = Loadable::Ready(Arc::new(diff));
    let blame = Arc::new(vec![gitcomet_core::services::BlameLine {
        commit_id: Arc::from("1111111111111111111111111111111111111111"),
        author: Arc::from("Ada"),
        author_time_unix: Some(1_700_000_000),
        summary: Arc::from("initial"),
        body: None,
        line: "let x = 1;".to_string(),
        prior_exists: true,
        source_path: None,
        prior_commit: None,
    }]);
    repo_state.history_state.blame_path = Some(PathBuf::from("src/lib.rs"));
    repo_state.history_state.blame_source = Some(gitcomet_core::domain::BlameSource::WorkingTree(
        gitcomet_core::domain::DiffArea::Unstaged,
    ));
    repo_state.history_state.blame = Loadable::Ready(Arc::clone(&blame));
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));
    (state, target, blame)
}

fn unstaged_diff(line: &str) -> gitcomet_core::domain::Diff {
    gitcomet_core::domain::Diff {
        target: DiffTarget::WorkingTree {
            path: PathBuf::from("src/lib.rs"),
            area: gitcomet_core::domain::DiffArea::Unstaged,
        },
        lines: vec![gitcomet_core::domain::DiffLine {
            kind: gitcomet_core::domain::DiffLineKind::Context,
            text: line.into(),
        }],
    }
}

#[test]
fn diff_loaded_identical_content_skips_rev_bumps_and_keeps_blame() {
    // A refresh that found no change must not churn the UI: window focus (and,
    // before the fix, every window drag) reloads the working-tree diff, and
    // bumping the revs would blank and re-run the expensive blame.
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
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
        matches!(&repo_state.history_state.blame, Loadable::Ready(lines) if Arc::ptr_eq(lines, &blame)),
        "blame must survive a reload that found no change"
    );
}

#[test]
fn diff_loaded_changed_content_bumps_revs_and_invalidates_blame() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
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
        matches!(repo_state.history_state.blame, Loadable::NotLoaded),
        "blame is derived from the diff content, so changed content invalidates it"
    );
    // The target is preserved so the view reloads the same file's blame, and the
    // outgoing annotations stay painted meanwhile.
    assert_eq!(
        repo_state.history_state.blame_path.as_deref(),
        Some(std::path::Path::new("src/lib.rs"))
    );
    assert!(
        repo_state
            .history_state
            .retained_blame_while_loading
            .as_ref()
            .is_some_and(|held| Arc::ptr_eq(held, &blame))
    );
}

#[test]
fn diff_loaded_error_after_ready_still_bumps_revs() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
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

fn file_diff_text(line: &str) -> gitcomet_core::domain::FileDiffText {
    gitcomet_core::domain::FileDiffText::new(
        PathBuf::from("icon.svg"),
        None,
        Some(line.to_string()),
    )
}

/// A repo showing a file-text view (`load_file_text`, not `load_patch_diff`)
/// with blame loaded — the shape that makes `diff_file` the only content signal.
fn state_with_loaded_file_text_and_blame(
    text: gitcomet_core::domain::FileDiffText,
) -> (
    AppState,
    DiffTarget,
    Arc<Vec<gitcomet_core::services::BlameLine>>,
) {
    let target = DiffTarget::WorkingTree {
        path: PathBuf::from("icon.svg"),
        area: DiffArea::Unstaged,
    };
    let (mut state, _, blame) = state_with_loaded_diff_and_blame(gitcomet_core::domain::Diff {
        target: target.clone(),
        lines: vec![],
    });
    state.repos[0].diff_state.diff = Loadable::NotLoaded;
    state.repos[0].diff_state.diff_file = Loadable::Ready(Some(Arc::new(text)));
    (state, target, blame)
}

#[test]
fn diff_file_loaded_identical_content_skips_rev_bumps_and_keeps_blame() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
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
        matches!(&repo_state.history_state.blame, Loadable::Ready(lines) if Arc::ptr_eq(lines, &blame))
    );
}

#[test]
fn diff_file_loaded_changed_content_bumps_revs_and_invalidates_blame() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
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
    assert!(matches!(
        repo_state.history_state.blame,
        Loadable::NotLoaded
    ));
}

#[test]
fn diff_file_loaded_and_image_loaded_cover_success_and_error_paths() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    let target = DiffTarget::WorkingTree {
        path: PathBuf::from("icon.svg"),
        area: DiffArea::Unstaged,
    };
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
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    let selected = DiffTarget::WorkingTree {
        path: PathBuf::from("selected.txt"),
        area: DiffArea::Unstaged,
    };
    let other = DiffTarget::WorkingTree {
        path: PathBuf::from("other.txt"),
        area: DiffArea::Unstaged,
    };
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
fn open_file_content_sets_diff_target_and_content_preview() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let path = PathBuf::from("src/main.rs");
    let source = FileSource::WorkingDirectory;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileContent {
            repo_id: RepoId(1),
            source,
            path: path.clone(),
        },
    );

    let repo_state = state.repos.first().expect("repo state to exist");
    assert_eq!(
        repo_state.diff_state.diff_target,
        Some(DiffTarget::WorkingTree {
            path: path.clone(),
            area: DiffArea::Unstaged,
        })
    );
    assert!(repo_state.diff_state.content_preview);
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::LoadSelectedDiff {
            repo_id: RepoId(1),
            ..
        }
    )));

    // Test with Commit source
    let commit_path = PathBuf::from("src/commit.rs");
    let commit_id = CommitId("deadbeef".into());
    let _effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileContent {
            repo_id: RepoId(1),
            source: FileSource::Commit(commit_id.clone()),
            path: commit_path.clone(),
        },
    );

    let repo_state = state.repos.first().expect("repo state to exist");
    assert_eq!(
        repo_state.diff_state.diff_target,
        Some(DiffTarget::Commit {
            commit_id,
            path: Some(commit_path),
        })
    );
    assert!(repo_state.diff_state.content_preview);
    assert!(repo_state.diff_state.diff_preview_text_file.is_loading());
    // Opening a file for reading never turns editing on.
    assert!(!repo_state.diff_state.edit_mode);
}

#[test]
fn open_file_editor_targets_the_working_tree_from_any_source() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    let path = PathBuf::from("src/main.rs");

    // Start on a commit's copy of the file...
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileContent {
            repo_id,
            source: FileSource::Commit(CommitId("deadbeef".into())),
            path: path.clone(),
        },
    );

    // ...and editing still opens the file on disk, not the historical blob.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileEditor {
            repo_id,
            path: path.clone(),
        },
    );

    let repo_state = state.repos.first().expect("repo state to exist");
    assert_eq!(
        repo_state.diff_state.diff_target,
        Some(DiffTarget::WorkingTree {
            path: path.clone(),
            area: DiffArea::Unstaged,
        })
    );
    assert!(repo_state.diff_state.content_preview);
    assert!(repo_state.diff_state.edit_mode);
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::LoadSelectedDiff {
            repo_id: RepoId(1),
            ..
        }
    )));
}

#[test]
fn selecting_another_view_leaves_edit_mode() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    let path = PathBuf::from("src/main.rs");
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileEditor {
            repo_id,
            path: path.clone(),
        },
    );
    assert!(state.repos[0].diff_state.edit_mode);

    // A plain diff selection is not an editing view.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id,
            target: DiffTarget::WorkingTree {
                path: PathBuf::from("other.rs"),
                area: DiffArea::Unstaged,
            },
        },
    );
    assert!(!state.repos[0].diff_state.edit_mode);
    assert!(!state.repos[0].diff_state.content_preview);

    // Neither is a read-only content view.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileEditor {
            repo_id,
            path: path.clone(),
        },
    );
    assert!(state.repos[0].diff_state.edit_mode);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileContent {
            repo_id,
            source: FileSource::WorkingDirectory,
            path,
        },
    );
    assert!(!state.repos[0].diff_state.edit_mode);
    assert!(state.repos[0].diff_state.content_preview);
}

#[test]
fn exiting_edit_mode_keeps_the_file_on_screen() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    let path = PathBuf::from("src/main.rs");
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileEditor {
            repo_id,
            path: path.clone(),
        },
    );
    assert!(state.repos[0].diff_state.edit_mode);
    let rev_before = state.repos[0].diff_state.diff_state_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ExitDiffEditMode { repo_id },
    );
    assert!(!state.repos[0].diff_state.edit_mode);
    assert!(
        state.repos[0].diff_state.content_preview,
        "leaving the editor lands on the read-only view of the same file"
    );
    assert_ne!(state.repos[0].diff_state.diff_state_rev, rev_before);

    // Idempotent: nothing to leave, nothing to repaint.
    let rev = state.repos[0].diff_state.diff_state_rev;
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ExitDiffEditMode { repo_id },
    );
    assert_eq!(state.repos[0].diff_state.diff_state_rev, rev);
}

#[test]
fn exiting_edit_mode_restores_the_originating_diff_or_content_preview() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    let path = PathBuf::from("src/main.rs");
    let commit_id = CommitId("deadbeef".into());
    let commit_target = DiffTarget::Commit {
        commit_id: commit_id.clone(),
        path: Some(path.clone()),
    };

    // A diff remains a diff, including its historical target, after editing
    // the working-tree copy.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id,
            target: commit_target.clone(),
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileEditor {
            repo_id,
            path: path.clone(),
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ExitDiffEditMode { repo_id },
    );
    assert_eq!(state.repos[0].diff_state.diff_target, Some(commit_target));
    assert!(!state.repos[0].diff_state.content_preview);
    assert!(!state.repos[0].diff_state.edit_mode);

    // A full-content preview returns to that preview and its original commit,
    // rather than being forced into the working-tree preview.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileContent {
            repo_id,
            source: FileSource::Commit(commit_id.clone()),
            path: path.clone(),
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileEditor { repo_id, path },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ExitDiffEditMode { repo_id },
    );
    assert_eq!(
        state.repos[0].diff_state.diff_target,
        Some(DiffTarget::Commit {
            commit_id,
            path: Some(PathBuf::from("src/main.rs")),
        })
    );
    assert!(state.repos[0].diff_state.content_preview);
    assert!(!state.repos[0].diff_state.edit_mode);
}

#[test]
fn global_nav_realigns_viewer_history_onto_restored_file_view() {
    // Regression: a global (mouse) navigation that lands on a file-content view
    // must reposition the in-viewer file-version history (`view_history`) onto
    // the file now shown, so the viewer's prev/next-version buttons step relative
    // to it rather than a stale cursor left behind by earlier file opens.
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    // Open three files in the viewer: view_history = [a, b, c], cursor = 2.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileContent {
            repo_id,
            source: FileSource::WorkingDirectory,
            path: PathBuf::from("a.rs"),
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileContent {
            repo_id,
            source: FileSource::Commit(CommitId("c1".into())),
            path: PathBuf::from("b.rs"),
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileContent {
            repo_id,
            source: FileSource::Commit(CommitId("c2".into())),
            path: PathBuf::from("c.rs"),
        },
    );
    assert_eq!(state.repos[0].navigation.view_history.cursor, 2);

    // Leave the viewer for a full-tree commit diff (not a file-content view): the
    // global stack records it, but view_history stops tracking and stays at c.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id,
            target: DiffTarget::Commit {
                commit_id: CommitId("c3".into()),
                path: None,
            },
        },
    );
    assert_eq!(
        state.repos[0].navigation.view_history.cursor, 2,
        "a non-content diff view must not record viewer history"
    );

    // Mouse-back twice: lands on c's content view, then b's content view.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::GlobalNavBack { repo_id },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::GlobalNavBack { repo_id },
    );

    // The viewer-history cursor now points at b (the file shown), not stale c, so
    // "previous version" would step to a and "next version" to c.
    let view_history = &state.repos[0].navigation.view_history;
    assert_eq!(view_history.cursor, 1);
    let current = &view_history.entries[view_history.cursor];
    assert_eq!(current.source, FileSource::Commit(CommitId("c1".into())));
    assert_eq!(current.path, PathBuf::from("b.rs"));
    assert!(view_history.can_back());
    assert!(view_history.can_forward());
}

#[test]
fn global_nav_reloads_commit_details_when_a_stale_load_is_in_flight() {
    // Regression: navigating back to a snapshot whose commit is already selected
    // makes `select_commit` no-op. If commit_details is still `Loading` for a
    // *different* (stale/cancelled) commit, the result will be dropped by the
    // id-guard, so trusting `is_loading()` would leave the details pane stuck
    // forever. global_nav must reload because the details shown are not for this
    // commit.
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    let commit_y = CommitId("yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy".into());
    // Two back/forward entries for the SAME commit (full diff, then one file), so
    // stepping back keeps `selected_commit` == Y and `select_commit` no-ops.
    let snap = |path: Option<PathBuf>| crate::model::MainViewSnapshot {
        diff_target: Some(DiffTarget::Commit {
            commit_id: commit_y.clone(),
            path,
        }),
        edit_mode: false,
        content_preview: false,
        selected_commit: Some(commit_y.clone()),
        range_selection: None,
        worktree_selection: None,
    };
    state.repos[0].navigation.main_history.record(snap(None));
    state.repos[0]
        .navigation
        .main_history
        .record(snap(Some(PathBuf::from("src/lib.rs"))));
    // Live view matches the nav tail; commit Y is selected but its details are
    // stuck Loading (the relevant load was cancelled / is for another commit).
    state.repos[0].diff_state.diff_target = Some(DiffTarget::Commit {
        commit_id: commit_y.clone(),
        path: Some(PathBuf::from("src/lib.rs")),
    });
    state.repos[0].set_selected_commit(Some(commit_y.clone()));
    state.repos[0].set_commit_details(Loadable::Loading);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::GlobalNavBack { repo_id },
    );

    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::LoadCommitDetails { repo_id: id, commit_id }
                if *id == repo_id && *commit_id == commit_y
        )),
        "stuck-Loading details for the restored commit must be reloaded, not skipped"
    );
    assert!(
        matches!(
            state.repos[0].history_state.commit_details,
            Loadable::NotLoaded
        ),
        "details are reset to NotLoaded before the reload"
    );
}

/// The comparison view takes precedence over both commit-detail views, so a
/// back/forward step that doesn't carry the comparison would restore a target
/// and selection the pane never gets around to showing. Stepping back out of a
/// comparison must leave it, and stepping forward into one must re-enter it —
/// including re-issuing the file-list load, since leaving dropped the list.
#[test]
fn global_nav_enters_and_leaves_a_range_comparison() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    let from = CommitId("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into());
    let to = CommitId("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into());
    let range = crate::model::RangeSelection {
        from: from.clone(),
        to: Some(to.clone()),
        from_label: "base".into(),
        to_label: "tip".into(),
    };

    // Step 0: the history log. Step 1: a comparison (which clears the diff pane).
    state.repos[0]
        .navigation
        .main_history
        .record(crate::model::MainViewSnapshot {
            diff_target: None,
            content_preview: false,
            edit_mode: false,
            selected_commit: None,
            range_selection: None,
            worktree_selection: None,
        });
    state.repos[0]
        .navigation
        .main_history
        .record(crate::model::MainViewSnapshot {
            diff_target: None,
            content_preview: false,
            edit_mode: false,
            selected_commit: None,
            range_selection: Some(range.clone()),
            worktree_selection: None,
        });
    // Live view matches the nav tail so the reduce-wrapper's reconcile no-ops.
    state.repos[0].set_range_selection(Some(range.clone()));

    // Back to the history log: the comparison must dissolve, not linger.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::GlobalNavBack { repo_id },
    );
    assert!(
        state.repos[0].history_state.range_selection.is_none(),
        "stepping back past a comparison must leave it"
    );

    // Forward into it again: restored, and its file list re-requested because
    // leaving dropped it.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::GlobalNavForward { repo_id },
    );
    assert_eq!(
        state.repos[0].history_state.range_selection,
        Some(range),
        "stepping forward into a comparison must restore its endpoints"
    );
    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::LoadRangeFiles { from: f, to: t, .. } if *f == from && *t == Some(to.clone())
        )),
        "the restored comparison must reload its changed-file list"
    );
}

#[test]
fn open_file_content_skips_conflict_path_during_browse() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    // Set up a conflict so that normal SelectDiff would enter conflict-loading
    let path = PathBuf::from("conflicted.txt");
    repo_state.staged_status = Loadable::Ready(Arc::new(vec![FileStatus {
        path: path.clone(),
        kind: FileStatusKind::Conflicted,
        conflict: Some(FileConflictKind::BothModified),
    }]));
    repo_state.worktree_status = Loadable::Ready(Arc::new(vec![FileStatus {
        path: path.clone(),
        kind: FileStatusKind::Conflicted,
        conflict: Some(FileConflictKind::BothModified),
    }]));
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileContent {
            repo_id: RepoId(1),
            source: FileSource::WorkingDirectory,
            path: path.clone(),
        },
    );

    let repo_state = state.repos.first().expect("repo state to exist");
    assert_eq!(
        repo_state.diff_state.diff_target,
        Some(DiffTarget::WorkingTree {
            path,
            area: DiffArea::Unstaged,
        })
    );
    assert!(repo_state.diff_state.content_preview);
    // effects should contain LoadSelectedDiff, NOT conflict-loading effects
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadSelectedDiff { .. }))
    );
}

#[test]
fn clear_diff_selection_resets_content_preview_and_ancillary_fields() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.diff_state.diff_target = Some(DiffTarget::WorkingTree {
        path: PathBuf::from("src/lib.rs"),
        area: DiffArea::Unstaged,
    });
    repo_state.diff_state.content_preview = true;
    repo_state.diff_state.diff = Loadable::Loading;
    repo_state.diff_state.diff_file = Loadable::Loading;
    repo_state.diff_state.diff_preview_text_file = Loadable::Loading;
    repo_state.diff_state.submodule_summary = Loadable::Loading;
    repo_state.diff_state.diff_file_image = Loadable::Loading;
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ClearDiffSelection { repo_id: RepoId(1) },
    );

    let repo_state = state.repos.first().expect("repo state to exist");
    assert!(repo_state.diff_state.diff_target.is_none());
    assert!(!repo_state.diff_state.content_preview);
    assert!(matches!(repo_state.diff_state.diff, Loadable::NotLoaded));
    assert!(matches!(
        repo_state.diff_state.diff_file,
        Loadable::NotLoaded
    ));
    assert!(matches!(
        repo_state.diff_state.diff_preview_text_file,
        Loadable::NotLoaded
    ));
    assert!(matches!(
        repo_state.diff_state.submodule_summary,
        Loadable::NotLoaded
    ));
    assert!(matches!(
        repo_state.diff_state.diff_file_image,
        Loadable::NotLoaded
    ));
    assert!(effects.is_empty());
}

#[test]
fn select_conflict_diff_sets_target_and_resets_content_preview() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.diff_state.content_preview = true;
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));

    let path = PathBuf::from("conflicted.txt");
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectConflictDiff {
            repo_id: RepoId(1),
            path: path.clone(),
        },
    );

    let repo_state = state.repos.first().expect("repo state to exist");
    assert!(!repo_state.diff_state.content_preview);
    assert_eq!(
        repo_state.diff_state.diff_target,
        Some(DiffTarget::WorkingTree {
            path,
            area: DiffArea::Unstaged,
        })
    );
    assert!(matches!(repo_state.diff_state.diff, Loadable::NotLoaded));
    assert!(matches!(
        repo_state.diff_state.diff_file,
        Loadable::NotLoaded
    ));
    assert!(matches!(
        repo_state.diff_state.diff_file_image,
        Loadable::NotLoaded
    ));
    assert!(
        repo_state
            .conflict_state
            .conflict_file_path
            .as_ref()
            .is_some()
    );
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::LoadSelectedConflictFile {
            repo_id: RepoId(1),
            ..
        }
    )));
}

#[test]
fn diff_file_image_loaded_drops_old_side_when_content_preview() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    let path = PathBuf::from("img.png");
    let target = DiffTarget::WorkingTree {
        path: path.clone(),
        area: DiffArea::Unstaged,
    };
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

#[test]
fn global_nav_steps_into_and_out_of_edit_mode() {
    // Opening the editor on the file already on screen changes nothing the
    // snapshot used to record, so it deduped away and back/forward could not
    // cross the boundary in either direction.
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    let path = PathBuf::from("src/lib.rs");

    // Read-only content view, then the editor on the same file, then back out.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileContent {
            repo_id,
            source: gitcomet_core::domain::FileSource::WorkingDirectory,
            path: path.clone(),
        },
    );
    assert!(state.repos[0].diff_state.content_preview);
    assert!(!state.repos[0].diff_state.edit_mode);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileEditor {
            repo_id,
            path: path.clone(),
        },
    );
    assert!(
        state.repos[0].diff_state.edit_mode,
        "the editor is open on the file"
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ExitDiffEditMode { repo_id },
    );
    assert!(!state.repos[0].diff_state.edit_mode);

    // Back returns to the editor rather than skipping over it.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::GlobalNavBack { repo_id },
    );
    assert!(
        state.repos[0].diff_state.edit_mode,
        "Back from the read-only view must land in the editor it was left from"
    );

    // Back again leaves the editor for the read-only view it was entered from.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::GlobalNavBack { repo_id },
    );
    assert!(
        !state.repos[0].diff_state.edit_mode,
        "Back out of the editor must return to the read-only content view"
    );
    assert!(
        state.repos[0].diff_state.content_preview,
        "and that view is the file's content, not a diff"
    );

    // Forward walks the same boundary in the other direction.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::GlobalNavForward { repo_id },
    );
    assert!(
        state.repos[0].diff_state.edit_mode,
        "Forward must step back into the editor"
    );
}

/// The inline foreign diff belongs to the worktree whose file list opened it.
/// Selecting a different worktree's row must retire it, or the diff pane keeps
/// showing the previous checkout's file — chip and all — beside a file list that
/// has nothing highlighted.
#[test]
fn selecting_another_worktree_retires_the_previous_worktrees_inline_diff() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let worktree_a = PathBuf::from("/tmp/wt/a");
    let worktree_b = PathBuf::from("/tmp/wt/b");
    let inline_target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: PathBuf::from("src/lib.rs"),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };

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
        }],
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
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let worktree = PathBuf::from("/tmp/wt/a");
    let commit_target = gitcomet_core::domain::DiffTarget::Commit {
        commit_id: CommitId("c0".into()),
        path: Some(PathBuf::from("src/main.rs")),
    };
    let inline_target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: PathBuf::from("src/lib.rs"),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };

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
        }],
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
    };

    for (label, result) in [(
        "a scan that no longer lists it",
        Ok(vec![dirty("/tmp/wt/other")]),
    )] {
        let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
        let id_alloc = AtomicU64::new(2);
        let mut state = AppState::default();
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

    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
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

    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
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
                entries,
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
        let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
        let id_alloc = AtomicU64::new(2);
        reduce(
            &mut repos,
            &id_alloc,
            state,
            Msg::Internal(crate::msg::InternalMsg::WorktreeDirtyLoaded {
                repo_id: RepoId(1),
                result: Ok(vec![summary(files)]),
            }),
        );
    };

    let fresh_state = || {
        let mut state = AppState::default();
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

    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::WorktreeDirtyLoaded {
            repo_id: RepoId(1),
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

    let mut state = AppState::default();
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
        entries,
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

    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::WorktreeDirtyLoaded {
            repo_id: RepoId(1),
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

    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::WorktreeDirtyLoaded {
            repo_id: RepoId(1),
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

fn worktree_dirty_summary(
    files: &[&str],
    branch: &str,
) -> gitcomet_core::domain::WorktreeDirtySummary {
    use gitcomet_core::domain::{FileStatus, WorktreeDirtySummary};

    WorktreeDirtySummary {
        path: PathBuf::from("/tmp/wt/a"),
        head: Some(CommitId("tip".into())),
        branch: Some(branch.to_string()),
        detached: false,
        added: 0,
        modified: files.len(),
        deleted: 0,
        staged: Vec::new(),
        unstaged: files
            .iter()
            .map(|name| FileStatus {
                path: PathBuf::from(name),
                kind: FileStatusKind::Modified,
                conflict: None,
            })
            .collect(),
    }
}

/// A repo with one linked worktree, its row selected and one of its files open
/// in the inline diff.
fn worktree_inline_diff_fixture(
    files: &[&str],
    selected_ix: usize,
    branch: &str,
) -> (AppState, PathBuf) {
    let worktree = PathBuf::from("/tmp/wt/a");
    let summary = worktree_dirty_summary(files, branch);
    let entries = crate::model::worktree_inline_diff_entries(&summary);
    let target = entries[selected_ix].target.clone();

    let mut state = AppState::default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.set_worktree_dirty(Loadable::Ready(vec![summary]));
    repo_state.history_state.worktree_selection = Some(worktree.clone());
    repo_state.diff_state.inline_submodule_diff = Some(crate::model::InlineSubmoduleDiffState {
        origin: crate::model::ForeignDiffOrigin::Worktree {
            branch: Some(branch.to_string()),
            detached: false,
        },
        submodule_repo_path: worktree.clone(),
        parent_submodule_path: worktree.clone(),
        entries,
        selected_ix,
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
    (state, worktree)
}

/// A pending history reveal re-drives on every render of the history panel, so
/// this message arrives once per frame until pagination reaches its target.
/// Re-running the body each time bumped `commit_details_rev` -- which the details
/// pane hashes, so the repaint drove the next render -- and re-armed a full
/// `git status` walk across every linked worktree.
#[test]
fn reselecting_the_same_worktree_row_is_a_no_op() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
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
    let inline_target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: PathBuf::from("src/lib.rs"),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };
    let state_with_open_worktree_diff = || {
        let mut state = AppState::default();
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
                }],
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
                repo_id: RepoId(1),
                commit_id: CommitId("tip".into()),
            },
        ),
        (
            "clearing the commit selection",
            Msg::ClearCommitSelection { repo_id: RepoId(1) },
        ),
        (
            "a scan that no longer lists the worktree",
            Msg::Internal(crate::msg::InternalMsg::WorktreeDirtyLoaded {
                repo_id: RepoId(1),
                result: Ok(Vec::<WorktreeDirtySummary>::new()),
            }),
        ),
        (
            "switching to another worktree",
            Msg::SelectWorktreeUncommitted {
                repo_id: RepoId(1),
                path: PathBuf::from("/tmp/wt/b"),
            },
        ),
    ];

    for (label, msg) in exits {
        let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
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
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = state_with_open_worktree_diff();
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectWorktreeUncommitted {
            repo_id: RepoId(1),
            path: worktree.clone(),
        },
    );
    assert!(
        state.repos[0].diff_state.inline_submodule_diff.is_some(),
        "re-selecting the same worktree must keep its open diff"
    );
}

/// A submodule inline diff has no worktree row behind it, so the worktree
/// invariant must not touch it.
#[test]
fn a_submodule_inline_diff_survives_the_worktree_invariant() {
    let submodule_path = PathBuf::from("/tmp/repo/vendor/submodule");
    let inline_target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: PathBuf::from("src/lib.rs"),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
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
        entries: Vec::new(),
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
        Msg::ClearCommitSelection { repo_id: RepoId(1) },
    );

    assert!(
        state.repos[0].diff_state.inline_submodule_diff.is_some(),
        "a submodule diff has no worktree selection and must be left alone"
    );
}

/// Only the selected worktree's changed files live in state, so selecting a row
/// has to ask for a scan that carries them — and that scan has to name the
/// worktree that was just selected, not whichever one was selected before.
#[test]
fn selecting_a_worktree_requests_a_scan_for_its_own_files() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
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
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
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
        staged: vec![
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
        ],
        unstaged: vec![],
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
    let other_target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: other.clone(),
        area: gitcomet_core::domain::DiffArea::Staged,
    };
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
