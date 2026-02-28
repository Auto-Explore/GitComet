use super::*;

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

    let target = gitgpui_core::domain::DiffTarget::WorkingTree {
        path: PathBuf::from("src/lib.rs"),
        area: gitgpui_core::domain::DiffArea::Unstaged,
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
    assert_eq!(repo_state.diff_target, Some(target.clone()));
    assert!(repo_state.diff.is_loading());
    assert!(repo_state.diff_file.is_loading());
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

    let target = gitgpui_core::domain::DiffTarget::WorkingTree {
        path: PathBuf::from("img.png"),
        area: gitgpui_core::domain::DiffArea::Unstaged,
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
    assert_eq!(repo_state.diff_target, Some(target.clone()));
    assert!(repo_state.diff.is_loading());
    assert!(matches!(repo_state.diff_file, Loadable::NotLoaded));
    assert!(repo_state.diff_file_image.is_loading());
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

    let target = gitgpui_core::domain::DiffTarget::WorkingTree {
        path: PathBuf::from("icon.svg"),
        area: gitgpui_core::domain::DiffArea::Unstaged,
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
    assert_eq!(repo_state.diff_target, Some(target.clone()));
    assert!(repo_state.diff.is_loading());
    assert!(repo_state.diff_file.is_loading());
    assert!(repo_state.diff_file_image.is_loading());
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
    repo_state.diff_target = Some(DiffTarget::WorkingTree {
        path: PathBuf::from("a.txt"),
        area: gitgpui_core::domain::DiffArea::Unstaged,
    });
    repo_state.diff = Loadable::NotLoaded;
    repo_state.diff_file = Loadable::NotLoaded;
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoCommandFinished {
            repo_id: RepoId(1),
            command: crate::msg::RepoCommandKind::StageHunk,
            result: Ok(CommandOutput::default()),
        },
    );

    let repo_state = state.repos.iter().find(|r| r.id == RepoId(1)).unwrap();
    assert!(repo_state.diff.is_loading());
    assert!(repo_state.diff_file.is_loading());
    assert!(effects.iter().any(|e| {
        matches!(e, Effect::LoadDiff { repo_id: RepoId(1), target: DiffTarget::WorkingTree { path, area: gitgpui_core::domain::DiffArea::Unstaged } } if path == &PathBuf::from("a.txt"))
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
    repo_state.diff_target = Some(gitgpui_core::domain::DiffTarget::WorkingTree {
        path: PathBuf::from("src/lib.rs"),
        area: gitgpui_core::domain::DiffArea::Unstaged,
    });
    repo_state.diff = Loadable::Loading;
    repo_state.diff_file = Loadable::Loading;
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ClearDiffSelection { repo_id: RepoId(1) },
    );

    let repo_state = state.repos.first().expect("repo state to exist");
    assert!(repo_state.diff_target.is_none());
    assert!(matches!(repo_state.diff, Loadable::NotLoaded));
    assert!(matches!(repo_state.diff_file, Loadable::NotLoaded));
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
        area: gitgpui_core::domain::DiffArea::Unstaged,
    };
    repo_state.diff_target = Some(target.clone());
    repo_state.diff = Loadable::Loading;
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));

    let error = Error::new(ErrorKind::Backend("diff failed".to_string()));
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::DiffLoaded {
            repo_id: RepoId(1),
            target,
            result: Err(error),
        },
    );

    let repo_state = &state.repos[0];
    assert!(matches!(repo_state.diff, Loadable::Error(_)));
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

    let before = state.repos[0].diff_state_rev;

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
        state.repos[0].diff_state_rev > before,
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
    let before = state.repos[0].diff_state_rev;

    // Now clear
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ClearDiffSelection { repo_id: RepoId(1) },
    );

    assert!(
        state.repos[0].diff_state_rev > before,
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
    let log_before = state.repos[0].log_rev;

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
    assert_eq!(state.repos[0].log_rev, log_before);
}
