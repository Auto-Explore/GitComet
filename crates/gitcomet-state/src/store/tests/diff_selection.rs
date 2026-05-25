use super::*;
use gitcomet_core::domain::{
    CommitDetails, CommitFileChange, CommitId, FileConflictKind, FileStatus, FileStatusKind,
    Submodule, SubmoduleDiffSummary, SubmoduleDiffSummaryMode, SubmoduleInnerChange,
    SubmoduleStatus,
};

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

    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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

    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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

#[test]
fn select_diff_for_staged_deleted_head_gitlink_loads_submodule_summary() {
    let dir = tempfile::tempdir().expect("tempdir");
    run_git(dir.path(), &["init", "-q"]);
    run_git(dir.path(), &["config", "commit.gpgsign", "false"]);
    run_git(dir.path(), &["config", "user.name", "Test User"]);
    run_git(dir.path(), &["config", "user.email", "test@example.com"]);
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

    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(2);
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
    repo_state.set_submodules(Loadable::Ready(Vec::new()));
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
fn select_diff_for_deleted_commit_file_skips_patch_diff_and_loads_file_preview() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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
        committed_at: "2026-04-07T12:00:00Z".to_string(),
        parent_ids: vec![],
        files: vec![CommitFileChange {
            path: PathBuf::from("report.json"),
            kind: FileStatusKind::Deleted,
            is_submodule: false,
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

    let target = gitcomet_core::domain::DiffTarget::CommitRange {
        from_commit_id: CommitId("aaaa".into()),
        to_commit_id: CommitId("bbbb".into()),
        path: Some(PathBuf::from("src/lib.rs")),
    };
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenInlineSubmoduleDiff {
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
        path: PathBuf::from("icons/logo.svg"),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenInlineSubmoduleDiff {
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

    let target = gitcomet_core::domain::DiffTarget::CommitRange {
        from_commit_id: CommitId("aaaa".into()),
        to_commit_id: CommitId("bbbb".into()),
        path: Some(PathBuf::from("src/lib.rs")),
    };
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenInlineSubmoduleDiff {
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

    let target = gitcomet_core::domain::DiffTarget::CommitRange {
        from_commit_id: CommitId("aaaa".into()),
        to_commit_id: CommitId("bbbb".into()),
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
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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
                committed_at: "2026-04-07T12:00:00Z".to_string(),
                parent_ids: vec![],
                files: vec![CommitFileChange {
                    path: PathBuf::from("report.json"),
                    kind: FileStatusKind::Deleted,
                    is_submodule: false,
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
        [Effect::LoadSelectedConflictFile {
            repo_id: RepoId(1),
            mode: crate::model::ConflictFileLoadMode::CurrentOnly
        }]
    ));
}

#[test]
fn select_diff_for_conflicted_svg_prefers_conflict_loader_over_preview_effects() {
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
fn select_and_clear_diff_update_diff_target_rev_only_when_target_changes() {
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
