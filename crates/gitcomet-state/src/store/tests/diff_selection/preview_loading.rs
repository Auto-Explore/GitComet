use super::*;

#[test]
fn select_diff_sets_loading_and_emits_effect() {
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
        PathBuf::from("src/lib.rs"),
        gitcomet_core::domain::DiffArea::Unstaged,
    );

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
        PathBuf::from("img.png"),
        gitcomet_core::domain::DiffArea::Unstaged,
    );

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
        PathBuf::from("app.ico"),
        gitcomet_core::domain::DiffArea::Unstaged,
    );

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
        PathBuf::from("icon.svg"),
        gitcomet_core::domain::DiffArea::Unstaged,
    );

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
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    let target = gitcomet_core::domain::DiffTarget::working_tree(
        PathBuf::from("report.json"),
        gitcomet_core::domain::DiffArea::Unstaged,
    );
    repo_state.set_status(Loadable::Ready(Arc::new(RepoStatus {
        unstaged: std::sync::Arc::new(vec![FileStatus {
            path: PathBuf::from("report.json"),
            kind: FileStatusKind::Untracked,
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
        PathBuf::from("report.json"),
        gitcomet_core::domain::DiffArea::Unstaged,
    );
    repo_state.set_status(Loadable::Ready(Arc::new(RepoStatus {
        unstaged: std::sync::Arc::new(vec![FileStatus {
            path: PathBuf::from("report.json"),
            kind: FileStatusKind::Deleted,
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
fn select_diff_for_deleted_commit_file_skips_patch_diff_and_loads_file_preview() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    let commit_id = CommitId("deadbeef".into());
    let target =
        gitcomet_core::domain::DiffTarget::commit(commit_id.clone(), PathBuf::from("report.json"));
    repo_state.history_state.commit_details = Loadable::Ready(Arc::new(CommitDetails {
        id: commit_id,
        message: "remove report".to_string(),
        author_name: String::new(),
        author_email: String::new(),
        authored_at_unix: 0,
        committed_at: "2026-04-07T12:00:00Z".to_string(),
        committed_at_unix: 0,
        parent_ids: vec![],
        files: vec![CommitFileChange::new(
            PathBuf::from("report.json"),
            FileStatusKind::Deleted,
        )],
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
fn commit_details_loaded_replans_selected_deleted_commit_file_to_preview_text_file() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    let commit_id = CommitId("deadbeef".into());
    let target =
        gitcomet_core::domain::DiffTarget::commit(commit_id.clone(), PathBuf::from("report.json"));
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
                files: vec![CommitFileChange::new(
                    PathBuf::from("report.json"),
                    FileStatusKind::Deleted,
                )],
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
    assert!(
        matches!(effects.as_slice(), [
        Effect::LoadDiffPreviewTextFile {
            repo_id: RepoId(1), target: effect_target,
            side: gitcomet_core::domain::DiffPreviewTextSide::Old,
        },
    ] if effect_target == &target),
        "got {effects:?}"
    );

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
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    let target = gitcomet_core::domain::DiffTarget::working_tree(
        PathBuf::from("index.html"),
        gitcomet_core::domain::DiffArea::Unstaged,
    );
    repo_state.set_status(Loadable::Ready(Arc::new(RepoStatus {
        unstaged: std::sync::Arc::new(vec![FileStatus {
            path: PathBuf::from("index.html"),
            kind: FileStatusKind::Conflicted,
            conflict: Some(FileConflictKind::BothModified),
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
        [
            Effect::LoadSelectedConflictFile {
                request: _,
                repo_id: RepoId(1),
                mode: crate::model::ConflictFileLoadMode::CurrentOnly
            },
            Effect::LoadSelectedDiff {
                load_patch_diff: false,
                load_file_text: false,
                preview_text_side: None,
                load_submodule_summary: false,
                load_file_image: false,
                ..
            }
        ]
    ));
}

#[test]
fn select_diff_for_conflicted_svg_prefers_conflict_loader_over_preview_effects() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    let target = gitcomet_core::domain::DiffTarget::working_tree(
        PathBuf::from("icon.svg"),
        gitcomet_core::domain::DiffArea::Unstaged,
    );
    repo_state.set_status(Loadable::Ready(Arc::new(RepoStatus {
        unstaged: std::sync::Arc::new(vec![FileStatus {
            path: PathBuf::from("icon.svg"),
            kind: FileStatusKind::Conflicted,
            conflict: Some(FileConflictKind::BothModified),
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
    assert!(matches!(
        repo_state.diff_state.diff_file_image,
        Loadable::NotLoaded
    ));
    assert!(matches!(
        effects.as_slice(),
        [
            Effect::LoadSelectedConflictFile {
                request: _,
                repo_id: RepoId(1),
                mode: crate::model::ConflictFileLoadMode::CurrentOnly
            },
            Effect::LoadSelectedDiff {
                load_patch_diff: false,
                load_file_text: false,
                preview_text_side: None,
                load_submodule_summary: false,
                load_file_image: false,
                ..
            }
        ]
    ));
}

#[test]
fn select_diff_for_a_range_without_path_does_not_load_a_whole_patch() {
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
        CommitId("deadbeef".into()),
        Some(CommitId("feedface".into())),
        None,
    );

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
        [Effect::LoadSelectedDiff {
            repo_id: RepoId(1),
            load_patch_diff: false,
            load_file_text: false,
            load_file_image: false,
            load_submodule_summary: false,
            preview_text_side: None,
        }]
    ));
}

#[test]
fn select_diff_for_commit_svg_path_loads_text_and_image_previews() {
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

    let target = gitcomet_core::domain::DiffTarget::commit(
        CommitId("deadbeef".into()),
        PathBuf::from("diagram.svg"),
    );

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
        Msg::StageHunk {
            repo_id: RepoId(1),
            patch: "diff --git a/a.txt b/a.txt\n".to_string().into(),
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::StageHunk {
            repo_id: RepoId(1),
            patch: _
        })]
    ));
}

#[test]
fn unstage_hunk_emits_effect() {
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
        Msg::UnstageHunk {
            repo_id: RepoId(1),
            patch: "diff --git a/a.txt b/a.txt\n".to_string().into(),
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::UnstageHunk {
            repo_id: RepoId(1),
            patch: _
        })]
    ));
}

#[test]
fn stage_hunk_command_finished_reloads_current_diff() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.diff_state.diff_target = Some(DiffTarget::working_tree(
        PathBuf::from("a.txt"),
        gitcomet_core::domain::DiffArea::Unstaged,
    ));
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
        matches!(e, Effect::LoadDiff { repo_id: RepoId(1), target: DiffTarget::WorkingTree { path, area: gitcomet_core::domain::DiffArea::Unstaged, .. } } if path == &PathBuf::from("a.txt"))
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
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    let target = DiffTarget::working_tree(
        PathBuf::from("a.txt"),
        gitcomet_core::domain::DiffArea::Unstaged,
    );
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
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
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
            target: DiffTarget::working_tree(
                PathBuf::from("b.txt"),
                gitcomet_core::domain::DiffArea::Unstaged,
            ),
        },
    );

    let repo_state = state.repos.iter().find(|r| r.id == RepoId(1)).unwrap();
    assert!(!repo_state.diff_state.diff_reload_in_flight);
}

#[test]
fn clear_diff_selection_resets_diff_state() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::working_tree(
        PathBuf::from("src/lib.rs"),
        gitcomet_core::domain::DiffArea::Unstaged,
    ));
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
fn clear_diff_selection_resets_content_preview_and_ancillary_fields() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.diff_state.diff_target = Some(DiffTarget::working_tree(
        PathBuf::from("src/lib.rs"),
        DiffArea::Unstaged,
    ));
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
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
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
    assert!(repo_state.diff_state.text_attributes.is_loading());
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadSelectedDiff {
            load_patch_diff: false,
            load_file_text: false,
            ..
        }
    )));
    assert!(!repo_state.diff_state.content_preview);
    assert_eq!(
        repo_state.diff_state.diff_target,
        Some(DiffTarget::working_tree(path, DiffArea::Unstaged))
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
            request: _,
            repo_id: RepoId(1),
            ..
        }
    )));
}
