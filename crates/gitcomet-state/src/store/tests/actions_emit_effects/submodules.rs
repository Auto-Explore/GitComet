use super::*;

#[test]
fn submodule_commands_reload_submodules_on_success() {
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
    state.repos[0].set_submodules(Loadable::Ready(Vec::new()));
    state.repos[0].submodule_add_in_flight = Some(crate::model::SubmoduleAddProgressState {
        url: "https://example.com/sub.git".to_string(),
        path: PathBuf::from("submodule"),
    });

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::AddSubmodule {
                url: "https://example.com/sub.git".to_string(),
                path: PathBuf::from("submodule"),
                branch: None,
                name: None,
                force: false,
                approved_sources: Vec::new(),
            },
            result: Ok(CommandOutput::empty_success("git submodule add")),
        }),
    );

    assert!(state.repos[0].submodules.is_loading());
    assert!(state.repos[0].submodule_add_in_flight.is_none());
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadSubmodules { repo_id: id } if *id == repo_id))
    );
    assert!(
        !state.repos[0]
            .loads_in_flight
            .finish(crate::model::LoadKind::Submodules)
    );

    state.repos[0].set_submodules(Loadable::Ready(Vec::new()));
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::UpdateSubmodules {
                approved_sources: Vec::new(),
            },
            result: Ok(CommandOutput::empty_success(
                "git submodule update --init --recursive",
            )),
        }),
    );

    assert!(state.repos[0].submodules.is_loading());
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadSubmodules { repo_id: id } if *id == repo_id))
    );
    assert!(
        !state.repos[0]
            .loads_in_flight
            .finish(crate::model::LoadKind::Submodules)
    );

    state.repos[0].set_submodules(Loadable::Ready(Vec::new()));
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::RemoveSubmodule {
                path: PathBuf::from("submodule"),
            },
            result: Ok(CommandOutput::empty_success(
                "git submodule deinit -f submodule",
            )),
        }),
    );

    assert!(state.repos[0].submodules.is_loading());
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadSubmodules { repo_id: id } if *id == repo_id))
    );
}

#[test]
fn selected_submodule_command_reloads_selected_summary() {
    use gitcomet_core::domain::{
        Diff, FileStatus, FileStatusKind, Submodule, SubmoduleDiffSummary,
        SubmoduleDiffSummaryMode, SubmoduleStatus,
    };

    fn seeded_state(
        repo_id: RepoId,
        command_path: &std::path::Path,
    ) -> (AppState, gitcomet_core::domain::DiffTarget) {
        let mut state = AppState::test_default();
        let mut repo = RepoState::new_opening(
            repo_id,
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        );
        repo.set_status(Loadable::Ready(Arc::new(RepoStatus {
            unstaged: std::sync::Arc::new(vec![FileStatus {
                path: command_path.to_path_buf(),
                kind: FileStatusKind::Modified,
                conflict: None,
            }]),
            staged: std::sync::Arc::new(Vec::new()),
        })));
        repo.set_submodules(Loadable::Ready(vec![Submodule {
            path: command_path.to_path_buf(),
            recorded_head: CommitId("old-recorded".into()),
            checked_out_head: None,
            status: SubmoduleStatus::NotInitialized,
        }]));
        let target = DiffTarget::working_tree(command_path.to_path_buf(), DiffArea::Unstaged);
        repo.diff_state.diff_target = Some(target.clone());
        repo.diff_state.submodule_summary = Loadable::Ready(Arc::new(SubmoduleDiffSummary {
            path: command_path.to_path_buf(),
            mode: SubmoduleDiffSummaryMode::Worktree,
            status: Some(SubmoduleStatus::NotInitialized),
            checkout_available: true,
            commit_id: None,
            parent_commit_id: None,
            checked_out_head: None,
            ranges: Vec::new(),
            live_staged: Vec::new(),
            live_unstaged: Vec::new(),
        }));
        let inline_target = DiffTarget::working_tree(PathBuf::from("inner.rs"), DiffArea::Unstaged);
        repo.diff_state.inline_submodule_diff = Some(crate::model::InlineSubmoduleDiffState {
            origin: crate::model::ForeignDiffOrigin::Submodule,
            submodule_repo_path: PathBuf::from("/tmp/repo/vendor/lib"),
            parent_submodule_path: command_path.to_path_buf(),
            entries: vec![crate::model::InlineSubmoduleDiffEntry {
                path: PathBuf::from("inner.rs"),
                kind: FileStatusKind::Modified,
                target: inline_target.clone(),
                section: crate::model::InlineSubmoduleDiffSection::LiveUnstaged,
            }]
            .into(),
            selected_ix: 0,
            target: inline_target,
            rev: 1,
            diff: Loadable::Ready(Arc::new(Diff {
                target: target.clone(),
                lines: Vec::new(),
            })),
            diff_rev: 1,
            diff_file_rev: 1,
            diff_file: Loadable::NotLoaded,
            diff_file_image: Loadable::NotLoaded,
        });
        state.repos.push(repo);
        (state, target)
    }

    let repo_id = RepoId(1);
    let path = PathBuf::from("vendor/lib");
    for command in [
        RepoCommandKind::LoadSubmodule {
            path: path.clone(),
            approved_sources: Vec::new(),
        },
        RepoCommandKind::UpdateSubmodules {
            approved_sources: Vec::new(),
        },
        RepoCommandKind::ChangeSubmodulePointer {
            path: path.clone(),
            reference: "main".to_string(),
        },
    ] {
        let mut repos: RepositoryRuntimes = Default::default();
        let id_alloc = AtomicU64::new(1);
        let (mut state, target) = seeded_state(repo_id, path.as_path());

        let effects = reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command,
                result: Ok(CommandOutput::empty_success("git submodule command")),
            }),
        );

        assert!(state.repos[0].diff_state.submodule_summary.is_loading());
        assert!(state.repos[0].diff_state.inline_submodule_diff.is_none());
        assert!(effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadSubmoduleSummary {
                repo_id: id,
                target: effect_target,
            } if *id == repo_id && *effect_target == target
        )));
    }
}

#[test]
fn local_submodule_add_trust_prompt_confirms_into_add_effect() {
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

    let source = gitcomet_core::services::SubmoduleTrustTarget {
        submodule_path: PathBuf::from("mods/sub"),
        display_source: "../local-sub".to_string(),
        local_source_path: PathBuf::from("/tmp/local-sub"),
    };

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::SubmoduleAddTrustChecked {
            repo_id,
            url: "../local-sub".to_string(),
            path: PathBuf::from("mods/sub"),
            branch: Some("feature".to_string()),
            name: None,
            force: false,
            result: Ok(gitcomet_core::services::SubmoduleTrustDecision::Prompt {
                sources: vec![source.clone()],
            }),
        }),
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.submodule_trust_prompt,
        Some(crate::model::SubmoduleTrustPromptState {
            repo_id,
            operation: crate::model::SubmoduleTrustPromptOperation::Add {
                url: "../local-sub".to_string(),
                path: PathBuf::from("mods/sub"),
                branch: Some("feature".to_string()),
                name: None,
                force: false,
            },
            sources: vec![source.clone()],
        })
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ConfirmSubmoduleTrustPrompt,
    );
    assert!(state.submodule_trust_prompt.is_none());
    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::AddSubmodule {
            repo_id: RepoId(1),
            url,
            path,
            branch,
            approved_sources,
            ..
        })] if url == "../local-sub"
            && path == &PathBuf::from("mods/sub")
            && branch.as_deref() == Some("feature")
            && approved_sources == &vec![source]
    ));
    assert_eq!(
        state.repos[0].submodule_add_in_flight,
        Some(crate::model::SubmoduleAddProgressState {
            url: "../local-sub".to_string(),
            path: PathBuf::from("mods/sub"),
        })
    );
}

#[test]
fn submodule_trust_check_pending_marks_and_clears_around_the_check() {
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

    // Triggering the add marks a pending check so the UI can show a spinner.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AddSubmodule {
            repo_id,
            url: "../local-sub".to_string(),
            path: PathBuf::from("mods/sub"),
            branch: None,
            name: None,
            force: false,
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(
            RepoCommandEffect::CheckSubmoduleAddTrust {
                repo_id: RepoId(1),
                ..
            }
        )]
    ));
    assert_eq!(
        state.submodule_trust_check_pending,
        Some(crate::model::SubmoduleTrustCheckState {
            repo_id,
            operation: crate::model::SubmoduleTrustCheckOperation::Add,
        })
    );

    // The check resolving (here, into a prompt) clears the pending marker.
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::SubmoduleAddTrustChecked {
            repo_id,
            url: "../local-sub".to_string(),
            path: PathBuf::from("mods/sub"),
            branch: None,
            name: None,
            force: false,
            result: Ok(gitcomet_core::services::SubmoduleTrustDecision::Prompt {
                sources: vec![gitcomet_core::services::SubmoduleTrustTarget {
                    submodule_path: PathBuf::from("mods/sub"),
                    display_source: "../local-sub".to_string(),
                    local_source_path: PathBuf::from("/tmp/local-sub"),
                }],
            }),
        }),
    );
    assert!(state.submodule_trust_check_pending.is_none());
    assert!(state.submodule_trust_prompt.is_some());
}

#[test]
fn a_failed_submodule_trust_check_is_reported_on_its_repo() {
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
        Msg::Internal(crate::msg::InternalMsg::SubmoduleUpdateTrustChecked {
            repo_id,
            result: Err(Error::new(ErrorKind::Backend(
                "config unreadable".to_string(),
            ))),
        }),
    );

    assert!(effects.is_empty());
    let diagnostics = &state.repos[0].feedback.diagnostics;
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].kind, crate::model::DiagnosticKind::Error);
    assert!(
        diagnostics[0]
            .message
            .starts_with("Submodule trust check failed")
    );
    assert!(diagnostics[0].message.contains("config unreadable"));
}

#[test]
fn submodule_add_progress_starts_when_trust_check_proceeds() {
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
        Msg::Internal(crate::msg::InternalMsg::SubmoduleAddTrustChecked {
            repo_id,
            url: "https://example.com/sub.git".to_string(),
            path: PathBuf::from("mods/sub"),
            branch: None,
            name: Some("deps/sub".to_string()),
            force: true,
            result: Ok(gitcomet_core::services::SubmoduleTrustDecision::Proceed),
        }),
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::AddSubmodule {
            repo_id: RepoId(1),
            url,
            path,
            name,
            force,
            ..
        })] if url == "https://example.com/sub.git"
            && path == &PathBuf::from("mods/sub")
            && name.as_deref() == Some("deps/sub")
            && *force
    ));
    assert_eq!(
        state.repos[0].submodule_add_in_flight,
        Some(crate::model::SubmoduleAddProgressState {
            url: "https://example.com/sub.git".to_string(),
            path: PathBuf::from("mods/sub"),
        })
    );
}

#[test]
fn submodule_add_progress_clears_on_failed_add_command() {
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
    state.repos[0].submodule_add_in_flight = Some(crate::model::SubmoduleAddProgressState {
        url: "https://example.com/sub.git".to_string(),
        path: PathBuf::from("mods/sub"),
    });

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::AddSubmodule {
                url: "https://example.com/sub.git".to_string(),
                path: PathBuf::from("mods/sub"),
                branch: None,
                name: None,
                force: false,
                approved_sources: Vec::new(),
            },
            result: Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Backend("submodule add failed".to_string()),
            )),
        }),
    );

    assert!(state.repos[0].submodule_add_in_flight.is_none());
    assert!(
        !effects.iter().any(
            |effect| matches!(effect, Effect::LoadSubmodules { repo_id: id } if *id == repo_id)
        )
    );
}

#[test]
fn local_submodule_update_trust_prompt_cancels_cleanly() {
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

    let source = gitcomet_core::services::SubmoduleTrustTarget {
        submodule_path: PathBuf::from("mods/sub"),
        display_source: "../local-sub".to_string(),
        local_source_path: PathBuf::from("/tmp/local-sub"),
    };

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::SubmoduleUpdateTrustChecked {
            repo_id,
            result: Ok(gitcomet_core::services::SubmoduleTrustDecision::Prompt {
                sources: vec![source],
            }),
        }),
    );
    assert!(matches!(
        state.submodule_trust_prompt,
        Some(crate::model::SubmoduleTrustPromptState {
            repo_id: RepoId(1),
            operation: crate::model::SubmoduleTrustPromptOperation::Update,
            ..
        })
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CancelSubmoduleTrustPrompt,
    );
    assert!(effects.is_empty());
    assert!(state.submodule_trust_prompt.is_none());
}
