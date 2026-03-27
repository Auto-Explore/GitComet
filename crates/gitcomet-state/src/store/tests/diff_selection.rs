use super::*;
use gitcomet_core::domain::{FileConflictKind, FileStatus, FileStatusKind};

#[test]
fn select_diff_sets_loading_and_emits_effect() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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
        [
            Effect::LoadDiffFile { repo_id: RepoId(1), target: a },
            Effect::LoadDiff { repo_id: RepoId(1), target: b },
        ] if a == &target && b == &target
    ));
}

#[test]
fn select_diff_for_image_sets_loading_and_emits_effect() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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
        [
            Effect::LoadDiffFileImage { repo_id: RepoId(1), target: a },
            Effect::LoadDiff { repo_id: RepoId(1), target: b },
        ] if a == &target && b == &target
    ));
}

#[test]
fn select_diff_for_ico_sets_loading_and_emits_effect() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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
        [
            Effect::LoadDiffFileImage { repo_id: RepoId(1), target: a },
            Effect::LoadDiff { repo_id: RepoId(1), target: b },
        ] if a == &target && b == &target
    ));
}

#[test]
fn select_diff_for_svg_loads_image_and_text() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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
        [
            Effect::LoadDiffFileImage { repo_id: RepoId(1), target: a },
            Effect::LoadDiffFile { repo_id: RepoId(1), target: b },
            Effect::LoadDiff { repo_id: RepoId(1), target: c },
        ] if a == &target && b == &target && c == &target
    ));
}

#[test]
fn select_diff_for_conflicted_file_skips_patch_and_file_diff_loads() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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
        [Effect::LoadConflictFile {
            repo_id: RepoId(1),
            path,
            mode: crate::model::ConflictFileLoadMode::CurrentOnly
        }] if path == &PathBuf::from("index.html")
    ));
}

#[test]
fn stage_hunk_emits_effect() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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
fn clear_diff_selection_resets_diff_state() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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
            .diagnostics
            .iter()
            .any(|d| d.message.contains("diff failed"))
    );
}

// --- Revision counter regression tests ---

#[test]
fn select_diff_bumps_diff_state_rev() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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
fn clear_diff_selection_bumps_diff_state_rev() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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
    assert!(repo_state.diagnostics.is_empty());
}

#[test]
fn diff_file_loaded_and_image_loaded_cover_success_and_error_paths() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    let target = DiffTarget::WorkingTree {
        path: PathBuf::from("img.png"),
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
            result: Ok(Some(gitcomet_core::domain::FileDiffText {
                path: PathBuf::from("img.png"),
                old: Some("old".to_string()),
                new: Some("new".to_string()),
            })),
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
                path: PathBuf::from("img.png"),
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
            .diagnostics
            .iter()
            .any(|d| d.message.contains("text side-by-side failed"))
    );
    assert!(
        state.repos[0]
            .diagnostics
            .iter()
            .any(|d| d.message.contains("image preview failed"))
    );
}

#[test]
fn diff_results_are_ignored_for_non_matching_target() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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
