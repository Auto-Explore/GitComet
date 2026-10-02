//! Git LFS and git-annex command, support and queue behaviour.

use super::*;

fn large_file_fixture() -> (
    FxHashMap<RepoId, Arc<dyn GitRepository>>,
    AtomicU64,
    AppState,
    RepoId,
) {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let mut state = AppState::test_default();
    let repo_id = RepoId(41);
    repos.insert(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
    let mut repo_state = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.open = Loadable::Ready(());
    state.repos.push(repo_state);
    (repos, AtomicU64::new(1), state, repo_id)
}

#[test]
fn large_file_command_runs_as_a_local_action_and_lock_changes_reload_locks() {
    use gitcomet_core::large_files::LargeFileCommand;
    let (mut repos, id_alloc, mut state, repo_id) = large_file_fixture();
    let command = LargeFileCommand::LfsLock {
        paths: vec![PathBuf::from("art/hero.psd")],
    };

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RunLargeFileCommand {
            repo_id,
            command: command.clone(),
        },
    );
    assert!(
        matches!(effects.as_slice(), [Effect::RunLargeFileCommand { command: c, auth: None, .. }] if *c == command),
        "{effects:?}"
    );
    assert_eq!(state.repos[0].local_actions_in_flight, 1);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::LargeFile { command },
            result: Ok(CommandOutput::default()),
        }),
    );
    assert_eq!(state.repos[0].local_actions_in_flight, 0);
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadLfsLocks { .. })),
        "a lock change reloads the lock list: {effects:?}"
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::LoadLargeFileSupport { .. })),
        "a file lock does not change the support summary"
    );
}

#[test]
fn lfs_download_reloads_a_selected_historical_diff() {
    use gitcomet_core::large_files::LargeFileCommand;
    let (mut repos, id_alloc, mut state, repo_id) = large_file_fixture();
    let target = DiffTarget::commit(CommitId("abc123".into()), "a.bin".into());
    state.repos[0].set_diff_target(Some(target.clone()));
    state.repos[0].set_selected_commit(Some(CommitId("abc123".into())));
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::LargeFile {
                command: LargeFileCommand::LfsFetchAll,
            },
            result: Ok(CommandOutput::default()),
        }),
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadDiffFile { target: t, .. } if t == &target)),
        "downloaded objects must invalidate the historical content cache: {effects:?}"
    );
    assert!(
        effects.iter().any(|effect| matches!(effect,
            Effect::LoadCommitDetails { commit_id, .. } if commit_id.as_ref() == "abc123"
        )),
        "the commit details chips must refresh too"
    );
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::LargeFile {
                command: LargeFileCommand::AnnexGetKeys {
                    keys: vec!["WORM-s1-m1--missing".into(), "WORM-s1-m1--available".into()],
                },
            },
            result: Err(Error::new(ErrorKind::Backend(
                "first key unavailable".into(),
            ))),
        }),
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadDiffFile { target: got, .. } if got == &target)),
        "partial downloads must refresh the displayed sides too"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadCommitDetails { .. }))
    );
}

#[test]
fn lockable_patterns_load_locks_once_and_failures_stay_quiet() {
    let (mut repos, id_alloc, mut state, repo_id) = large_file_fixture();
    let mut support = gitcomet_core::large_files::LargeFileSupport::default();
    support.lfs.has_lockable_patterns = true;
    support
        .lfs
        .tracked_patterns
        .push(gitcomet_core::large_files::LfsTrackedPattern {
            pattern: "*.psd".into(),
            source: PathBuf::from(".gitattributes"),
        });
    let loaded = |support| {
        Msg::Internal(crate::msg::InternalMsg::LargeFileSupportLoaded {
            repo_id,
            result: Ok(support),
        })
    };
    let effects = reduce(&mut repos, &id_alloc, &mut state, loaded(support.clone()));
    assert_eq!(
        effects
            .iter()
            .filter(|e| matches!(e, Effect::LoadLfsLocks { .. }))
            .count(),
        1
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LfsLocksLoaded {
            repo_id,
            result: Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Backend("locking API not supported".into()),
            )),
        }),
    );
    assert!(effects.is_empty());
    assert!(matches!(state.repos[0].lfs_locks, Loadable::Error(_)));
    assert!(state.repos[0].feedback.diagnostics.is_empty());

    // Unchanged support does not ask the server again.
    let effects = reduce(&mut repos, &id_alloc, &mut state, loaded(support));
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::LoadLfsLocks { .. }))
    );
}

#[test]
fn support_changes_refresh_selected_content_but_unchanged_support_does_not() {
    use crate::msg::InternalMsg;
    let (mut repos, id_alloc, mut state, repo_id) = large_file_fixture();
    let commit_id = CommitId("abc123".into());
    let target = DiffTarget::commit(commit_id.clone(), "a.bin".into());
    state.repos[0].history_state.selected_commit = Some(commit_id);
    state.repos[0].diff_state.diff_target = Some(target.clone());
    let mut support = gitcomet_core::large_files::LargeFileSupport::default();
    support
        .lfs
        .tracked_patterns
        .push(gitcomet_core::large_files::LfsTrackedPattern {
            pattern: "*.bin".into(),
            source: ".gitattributes".into(),
        });
    let mut metadata_only = support.clone();
    metadata_only.annex.restage_pending = true;
    metadata_only.annex.numcopies = Some(2);
    metadata_only.annex.assistant_running = true;
    metadata_only
        .annex
        .repositories
        .push(gitcomet_core::large_files::AnnexRepository {
            uuid: "test".into(),
            description: "new description".into(),
            remote_name: None,
            special_type: None,
            special_name: None,
            trust: gitcomet_core::large_files::AnnexTrust::Untrusted,
            here: true,
        });
    for (support, refresh) in [
        (support.clone(), true),
        (support, false),
        (metadata_only, false),
        (Default::default(), true),
    ] {
        let effects = reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(InternalMsg::LargeFileSupportLoaded {
                repo_id,
                result: Ok(support),
            }),
        );
        assert_eq!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::LoadDiffFile { target: got, .. } if got == &target)),
            refresh
        );
        assert_eq!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::LoadCommitDetails { .. })),
            refresh
        );
    }
}

#[test]
fn support_storage_changes_reclassify_rows_before_or_after_an_old_scan_finishes() {
    use crate::msg::InternalMsg;
    use gitcomet_core::domain::{FileStatus, FileStatusKind};
    use gitcomet_core::large_files::{
        LargeFilePointer, LargeFileState, LargeFileSupport, UncommittedLargeFiles,
    };

    for old_scan_finishes_first in [true, false] {
        let (mut repos, id_alloc, mut state, repo_id) = large_file_fixture();
        let mut support = LargeFileSupport::default();
        support.lfs.has_local_store = true;
        support.lfs.storage_dir = "/tmp/repo/old-lfs".into();
        state.repos[0].set_large_file_support(Loadable::Ready(support.clone()));
        let status = Arc::new(RepoStatus {
            staged: Arc::new(vec![FileStatus {
                path: "a.bin".into(),
                kind: FileStatusKind::Added,
                conflict: None,
            }]),
            unstaged: Arc::default(),
        });
        state.repos[0].set_status(Loadable::Ready(status.clone()));
        let completed = |generation, present| {
            let mut rows = UncommittedLargeFiles::default();
            rows.staged.insert(
                "a.bin".into(),
                LargeFileState {
                    pointer: LargeFilePointer::Lfs(gitcomet_core::lfs::LfsPointer {
                        oid: gitcomet_core::lfs::LfsOid([1; 32]),
                        size: 12,
                    }),
                    in_local_store: Some(present),
                    worktree: None,
                    lockable: false,
                },
            );
            Msg::Internal(InternalMsg::UncommittedLineStatsLoaded {
                repo_id,
                generation,
                result: Ok(Default::default()),
                large_files: Some(Ok(rows)),
            })
        };
        state.repos[0].loads_in_flight.invalidate_line_stats();
        let old_generation = state.repos[0]
            .loads_in_flight
            .start_line_stats(true)
            .unwrap();
        if old_scan_finishes_first {
            assert!(
                reduce(
                    &mut repos,
                    &id_alloc,
                    &mut state,
                    completed(old_generation, false),
                )
                .is_empty()
            );
            assert!(
                state.repos[0].uncommitted_large_files.staged[Path::new("a.bin")].content_missing()
            );
        }

        // LFS remains active; only the object store changes.
        support.lfs.storage_dir = "/tmp/repo/new-lfs".into();
        let loaded = || {
            Msg::Internal(InternalMsg::LargeFileSupportLoaded {
                repo_id,
                result: Ok(support.clone()),
            })
        };
        let mut effects = reduce(&mut repos, &id_alloc, &mut state, loaded());
        if !old_scan_finishes_first {
            assert!(effects.is_empty(), "the old scan still holds the lane");
            let previous = state.repos[0].uncommitted_large_files.clone();
            effects = reduce(
                &mut repos,
                &id_alloc,
                &mut state,
                completed(old_generation, false),
            );
            assert_eq!(
                state.repos[0].uncommitted_large_files, previous,
                "discard the stale classification"
            );
        }
        let [
            Effect::LoadUncommittedLineStats {
                generation,
                status: replay_status,
                large_files: true,
                ..
            },
        ] = effects.as_slice()
        else {
            panic!("expected one row classification replay: {effects:?}");
        };
        assert_eq!(replay_status, &status);
        assert_ne!(*generation, old_generation);
        assert!(
            reduce(
                &mut repos,
                &id_alloc,
                &mut state,
                completed(*generation, true),
            )
            .is_empty()
        );
        assert_eq!(
            state.repos[0].uncommitted_large_files.staged[Path::new("a.bin")].in_local_store,
            Some(true)
        );
        assert!(
            reduce(&mut repos, &id_alloc, &mut state, loaded()).is_empty(),
            "unchanged support must not start another scan"
        );
    }
}

#[test]
fn large_file_command_replays_after_an_auth_prompt() {
    use gitcomet_core::large_files::LargeFileCommand;
    let command = LargeFileCommand::LfsFetchAll;
    let replay = crate::store::reducer::repo_command_replay_msg_for_test(
        RepoId(3),
        RepoCommandKind::LargeFile {
            command: command.clone(),
        },
    );
    assert!(
        matches!(
            &replay,
            Some(Msg::RunLargeFileCommand { repo_id: RepoId(3), command: c }) if *c == command
        ),
        "{replay:?}"
    );
}

/// On an adjusted branch a plain merge would commit adjusted content to the
/// wrong branch, so Pull and Push run git-annex's own pull and push.
#[test]
fn pull_and_push_on_adjusted_branch_use_git_annex() {
    use gitcomet_core::large_files::LargeFileCommand;
    let (mut repos, id_alloc, mut state, repo_id) = large_file_fixture();
    annex_repo_on(&mut state, "adjusted/main(unlocked)");

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Pull {
            repo_id,
            mode: PullMode::Default,
        },
    );
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::RunLargeFileCommand {
                command: LargeFileCommand::AnnexPull { content: false },
                ..
            }]
        ),
        "{effects:?}"
    );
    assert_eq!(state.repos[0].pull_in_flight, 1);
    assert_eq!(state.repos[0].worktree_pull_in_flight, 1);
    assert!(
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Pull {
                repo_id,
                mode: PullMode::Default,
            }
        )
        .is_empty(),
        "repeated Pull must not start another annex process"
    );

    state.large_file_settings.annex_sync_content = true;
    let effects = reduce(&mut repos, &id_alloc, &mut state, Msg::Push { repo_id });
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::RunLargeFileCommand {
                command: LargeFileCommand::AnnexPush { content: true },
                ..
            }]
        ),
        "{effects:?}"
    );

    state.large_file_settings.annex_pull_push = false;
    let effects = reduce(&mut repos, &id_alloc, &mut state, Msg::Push { repo_id });
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::RunLargeFileCommand { .. })),
        "turning the setting off restores plain push"
    );
}

#[test]
fn adjusted_branch_never_uses_plain_git_while_support_is_unknown() {
    for support in [
        Loadable::NotLoaded,
        Loadable::Loading,
        Loadable::Error("failed".into()),
        Loadable::Ready(Arc::default()),
    ] {
        let (mut repos, id_alloc, mut state, repo_id) = large_file_fixture();
        annex_repo_on(&mut state, "adjusted/main(unlocked)");
        state.repos[0].large_file_support = support;
        let confirmed_inactive = matches!(state.repos[0].large_file_support, Loadable::Ready(_));
        for message in [
            Msg::Pull {
                repo_id,
                mode: PullMode::Default,
            },
            Msg::Push { repo_id },
        ] {
            let effects = reduce(&mut repos, &id_alloc, &mut state, message);
            assert!(
                effects
                    .iter()
                    .any(|e| matches!(e, Effect::RunLargeFileCommand { .. }))
                    != confirmed_inactive,
                "{effects:?}"
            );
            assert!(
                effects
                    .iter()
                    .any(|e| matches!(e, Effect::Push { .. } | Effect::Pull { .. }))
                    == confirmed_inactive,
                "{effects:?}"
            );
        }
    }
}

#[test]
fn command_log_only_links_reportable_operations() {
    use crate::msg::InternalMsg;
    use gitcomet_core::git_operation::GitOperationId;
    for reportable in [false, true] {
        for success in [false, true] {
            let (mut repos, id_alloc, mut state, repo_id) = large_file_fixture();
            let operation_id = GitOperationId(4242);
            reduce(
                &mut repos,
                &id_alloc,
                &mut state,
                Msg::Internal(InternalMsg::GitOperationStarted {
                    repo_id,
                    operation_id,
                    label: "Push".into(),
                    context: None,
                    time: std::time::SystemTime::UNIX_EPOCH,
                    progress_lane: false,
                }),
            );
            if reportable {
                reduce(
                    &mut repos,
                    &id_alloc,
                    &mut state,
                    Msg::Internal(InternalMsg::GitOperationEvent {
                        repo_id,
                        operation_id,
                        event: gitcomet_core::git_operation::GitOperationEvent::CommandStarted,
                    }),
                );
            }
            reduce(
                &mut repos,
                &id_alloc,
                &mut state,
                Msg::Internal(InternalMsg::GitOperationFinished {
                    repo_id,
                    operation_id,
                    outer_outcome: if success {
                        crate::model::GitOperationOuterOutcome::Succeeded
                    } else {
                        crate::model::GitOperationOuterOutcome::Failed
                    },
                    duration: std::time::Duration::from_millis(1),
                    message: Box::new(InternalMsg::RepoCommandFinished {
                        repo_id,
                        command: RepoCommandKind::Push,
                        result: if success {
                            Ok(gitcomet_core::services::CommandOutput {
                                command: "git push".into(),
                                stdout: "file".into(),
                                stderr: String::new(),
                                exit_code: Some(0),
                            })
                        } else {
                            Err(gitcomet_core::error::Error::new(
                                gitcomet_core::error::ErrorKind::Backend("rejected push".into()),
                            ))
                        },
                    }),
                }),
            );
            let repo = &state.repos[0];
            assert_eq!(
                repo.feedback.command_log.last().unwrap().hook_operation_id,
                reportable.then_some(operation_id)
            );
            assert_eq!(repo.feedback.command_log.last().unwrap().ok, success);
            assert_eq!(repo.feedback.hook_activity.len(), usize::from(reportable));
            assert_eq!(repo.feedback.command_log_operation_id, None);
        }
    }
}

/// An annex repo whose support was loaded while HEAD was on `main`.
fn annex_repo_on(state: &mut AppState, head: &str) {
    let mut support = gitcomet_core::large_files::LargeFileSupport::default();
    support.annex.uuid = Some("u".into());
    support.annex.has_annex_branch = true;
    state.repos[0].large_file_support = Loadable::Ready(Arc::new(support));
    state.repos[0].head_branch = Loadable::Ready(head.to_string());
}

#[test]
fn review_commit_and_amend_push_wait_for_running_push() {
    use crate::msg::InternalMsg;
    for amend in [false, true] {
        let (mut repos, id_alloc, mut state, repo_id) = large_file_fixture();
        annex_repo_on(&mut state, "adjusted/main(unlocked)");
        reduce(&mut repos, &id_alloc, &mut state, Msg::Push { repo_id });
        state.repos[0].pending.commit_retry = Some(crate::model::PendingCommitRetry {
            message: "ship".into(),
            amend,
            push_after_commit: true,
        });
        let result = Ok(gitcomet_core::services::CommitOperationOutcome {
            local_branch: Some("adjusted/main(unlocked)".into()),
            pre_head: None,
            post_head: Some(CommitId("2222222222222222222222222222222222222222".into())),
        });
        let message = if amend {
            InternalMsg::CommitAmendFinished { repo_id, result }
        } else {
            InternalMsg::CommitFinished { repo_id, result }
        };
        let effects = reduce(&mut repos, &id_alloc, &mut state, Msg::Internal(message));
        assert!(!runs_annex(&effects, false));
        let effects = reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::LargeFile {
                    command: gitcomet_core::large_files::LargeFileCommand::AnnexPush {
                        content: false,
                    },
                },
                result: Ok(CommandOutput::default()),
            }),
        );
        assert!(
            runs_annex(&effects, false),
            "the push after {amend:?} must run once the previous push finishes: {effects:?}"
        );
        assert_eq!(state.repos[0].push_in_flight, 1);
        finish_large_file_effects(&mut repos, &id_alloc, &mut state, &effects);
        assert_eq!(state.repos[0].push_in_flight, 0);
    }
}

#[test]
fn review_annex_sync_is_not_blocked_by_fetch_or_prune() {
    use gitcomet_core::large_files::LargeFileCommand as C;
    for operation in 0..3 {
        for command in [
            C::AnnexPull { content: false },
            C::AnnexSync { content: true },
        ] {
            let (mut repos, id_alloc, mut state, repo_id) = large_file_fixture();
            let message = match operation {
                0 => Msg::Fetch(crate::msg::FetchMsg::All { repo_id }),
                1 => Msg::PruneMergedBranches { repo_id },
                _ => Msg::PruneLocalTags { repo_id },
            };
            reduce(&mut repos, &id_alloc, &mut state, message);
            let effects = reduce(
                &mut repos,
                &id_alloc,
                &mut state,
                Msg::RunLargeFileCommand {
                    repo_id,
                    command: command.clone(),
                },
            );
            assert!(effects.iter().any(|effect| matches!(effect, Effect::RunLargeFileCommand { command: actual, .. } if actual == &command)), "fetch/prune {operation} must not suppress {command:?}: {effects:?}");
            finish_large_file_effects(&mut repos, &id_alloc, &mut state, &effects);
            assert_eq!(state.repos[0].pull_in_flight, 1);
            assert_eq!(state.repos[0].worktree_pull_in_flight, 0);
            assert_eq!(state.repos[0].push_in_flight, 0);
        }
    }
}

#[test]
fn review_busy_annex_commands_provide_feedback_and_run_when_ready() {
    use gitcomet_core::large_files::LargeFileCommand as C;
    for command in [
        C::AnnexPull { content: false },
        C::AnnexPush { content: false },
        C::AnnexSync { content: true },
    ] {
        let (mut repos, id_alloc, mut state, repo_id) = large_file_fixture();
        let first = reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::RunLargeFileCommand {
                repo_id,
                command: C::AnnexSync { content: false },
            },
        );
        let effects = reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::RunLargeFileCommand {
                repo_id,
                command: command.clone(),
            },
        );
        assert!(effects.is_empty());
        assert!(
            !state.notifications.is_empty(),
            "a queued {command:?} must tell the user"
        );
        let Effect::RunLargeFileCommand {
            command: running, ..
        } = &first[0]
        else {
            panic!("{first:?}")
        };
        let effects = reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::LargeFile {
                    command: running.clone(),
                },
                result: Ok(CommandOutput::default()),
            }),
        );
        assert!(effects.iter().any(|effect| matches!(effect, Effect::RunLargeFileCommand { command: actual, .. } if actual == &command)), "{effects:?}");
    }
}

#[test]
fn queued_annex_push_runs_after_plain_push_success_failure_or_cancellation() {
    use gitcomet_core::large_files::LargeFileCommand as C;
    for error in [
        None,
        Some(ErrorKind::Cancelled),
        Some(ErrorKind::Backend("push failed".into())),
    ] {
        let (mut repos, id_alloc, mut state, repo_id) = large_file_fixture();
        reduce(&mut repos, &id_alloc, &mut state, Msg::Push { repo_id });
        let effects = reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::RunLargeFileCommand {
                repo_id,
                command: C::AnnexPush { content: true },
            },
        );
        assert!(effects.is_empty());
        assert_eq!(state.repos[0].pending.large_file_commands.len(), 1);
        let effects = reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::Push,
                result: match error {
                    Some(error) => Err(Error::new(error)),
                    None => Ok(CommandOutput::default()),
                },
            }),
        );
        assert!(state.repos[0].pending.large_file_commands.is_empty());
        assert!(runs_annex(&effects, false), "{effects:?}");
        assert_eq!(state.repos[0].push_in_flight, 1);
        assert_eq!(state.repos[0].local_actions_in_flight, 1);
        finish_large_file_effects(&mut repos, &id_alloc, &mut state, &effects);
        assert_eq!(state.repos[0].push_in_flight, 0);
        assert_eq!(state.repos[0].local_actions_in_flight, 0);
    }
}

#[test]
fn review_restage_and_lfs_downloads_rescan_support_in_inactive_repo() {
    use gitcomet_core::large_files::LargeFileCommand as C;
    for command in [
        C::AnnexRestage,
        C::LfsPull {
            paths: vec!["asset.bin".into()],
        },
        C::LfsFetchForDiff {
            target: DiffTarget::working_tree("asset.bin".into(), DiffArea::Unstaged),
        },
    ] {
        let (mut repos, id_alloc, mut state, repo_id) = large_file_fixture();
        assert_ne!(state.active_repo, Some(repo_id));
        let effects = reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::LargeFile {
                    command: command.clone(),
                },
                result: Ok(CommandOutput::default()),
            }),
        );
        assert!(effects.iter().any(|effect| matches!(effect, Effect::LoadLargeFileSupport { repo_id: actual } if *actual == repo_id)), "{command:?}: {effects:?}");
    }
}

#[test]
fn review_adjusted_merge_refusal_explains_merge_risk() {
    for squash in [false, true] {
        let (mut repos, id_alloc, mut state, repo_id) = large_file_fixture();
        annex_repo_on(&mut state, "adjusted/main(unlocked)");
        let message = if squash {
            Msg::SquashRef {
                repo_id,
                reference: "feature".into(),
            }
        } else {
            Msg::MergeRef {
                repo_id,
                reference: "feature".into(),
            }
        };
        assert!(reduce(&mut repos, &id_alloc, &mut state, message).is_empty());
        let reason = state.repos[0].feedback.last_error.as_ref().unwrap();
        assert!(
            reason.contains("adjusted content") && reason.contains("Check out main"),
            "{reason}"
        );
    }
}

fn runs_annex(effects: &[Effect], pull: bool) -> bool {
    use gitcomet_core::large_files::LargeFileCommand;
    effects.iter().any(|effect| {
        matches!(
            (effect, pull),
            (
                Effect::RunLargeFileCommand {
                    command: LargeFileCommand::AnnexPull { .. },
                    ..
                },
                true
            ) | (
                Effect::RunLargeFileCommand {
                    command: LargeFileCommand::AnnexPush { .. },
                    ..
                },
                false
            )
        )
    })
}

fn finish_large_file_effects(
    repos: &mut FxHashMap<RepoId, Arc<dyn GitRepository>>,
    id_alloc: &AtomicU64,
    state: &mut AppState,
    effects: &[Effect],
) {
    for effect in effects {
        if let Effect::RunLargeFileCommand {
            repo_id, command, ..
        } = effect
        {
            reduce(
                repos,
                id_alloc,
                state,
                Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
                    repo_id: *repo_id,
                    command: RepoCommandKind::LargeFile {
                        command: command.clone(),
                    },
                    result: Ok(CommandOutput::default()),
                }),
            );
        }
    }
}

/// Support is loaded once per repo open, but HEAD moves under it: a checkout
/// from the sidebar (a repo action) or a terminal. The takeover follows the
/// branch HEAD is on now, and a HEAD change reloads the support summary.
#[test]
fn adjusted_branch_takeover_follows_head_not_the_support_snapshot() {
    let (mut repos, id_alloc, mut state, repo_id) = large_file_fixture();
    annex_repo_on(&mut state, "main");
    let head_loaded = |head: &str| {
        Msg::Internal(crate::msg::InternalMsg::HeadBranchLoaded {
            repo_id,
            result: Ok(head.to_string()),
        })
    };

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        head_loaded("adjusted/main(unlocked)"),
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadLargeFileSupport { .. })),
        "a HEAD change reloads the support summary: {effects:?}"
    );
    let pull = || Msg::Pull {
        repo_id,
        mode: PullMode::Default,
    };
    let effects = reduce(&mut repos, &id_alloc, &mut state, pull());
    assert!(runs_annex(&effects, true), "{effects:?}");

    // Left the adjusted branch from a terminal: plain pull again.
    reduce(&mut repos, &id_alloc, &mut state, head_loaded("main"));
    let effects = reduce(&mut repos, &id_alloc, &mut state, pull());
    assert!(
        effects.iter().any(|e| matches!(e, Effect::Pull { .. })),
        "{effects:?}"
    );
}

/// Every way GitComet pulls into or pushes from the current branch must avoid
/// plain git on an adjusted branch: the ones git-annex has an equivalent for
/// go through it, the rest are refused with a reason.
#[test]
fn every_pull_and_push_path_on_an_adjusted_branch_avoids_plain_git() {
    let (mut repos, id_alloc, mut state, repo_id) = large_file_fixture();
    annex_repo_on(&mut state, "adjusted/main(unlocked)");
    let target = gitcomet_core::services::SafePushAfterCommitTarget {
        remote: "origin".to_string(),
        branch: "adjusted/main(unlocked)".to_string(),
        local_branch: "adjusted/main(unlocked)".to_string(),
        local_head: CommitId("2222222222222222222222222222222222222222".into()),
    };
    let context = gitcomet_core::services::SafePushAfterCommitContext {
        amend: false,
        local_branch: Some("adjusted/main(unlocked)".to_string()),
        pre_head: None,
        post_head: Some(CommitId("2222222222222222222222222222222222222222".into())),
    };

    // An adjusted branch has no upstream, so the toolbar offers set-upstream.
    for msg in [
        Msg::PushSetUpstream {
            repo_id,
            remote: "origin".into(),
            branch: "adjusted/main(unlocked)".into(),
        },
        Msg::PushAfterCommit {
            repo_id,
            target: target.clone(),
            set_upstream: true,
        },
        Msg::SafePushAfterCommit {
            repo_id,
            context: context.clone(),
        },
    ] {
        let label = format!("{msg:?}");
        let effects = reduce(&mut repos, &id_alloc, &mut state, msg);
        assert!(runs_annex(&effects, false), "{label}: {effects:?}");
        assert_eq!(state.repos[0].push_in_flight, 1);
        finish_large_file_effects(&mut repos, &id_alloc, &mut state, &effects);
    }

    state.repos[0].pending.commit_retry = Some(crate::model::PendingCommitRetry {
        message: "ship".to_string(),
        amend: false,
        push_after_commit: true,
    });
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitFinished {
            repo_id,
            result: Ok(gitcomet_core::services::CommitOperationOutcome {
                local_branch: context.local_branch.clone(),
                pre_head: None,
                post_head: context.post_head.clone(),
            }),
        }),
    );
    assert!(runs_annex(&effects, false), "commit & push: {effects:?}");
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::SafePushAfterCommit { .. })),
        "{effects:?}"
    );

    finish_large_file_effects(&mut repos, &id_alloc, &mut state, &effects);

    for msg in [
        Msg::PullBranch {
            repo_id,
            remote: "origin".into(),
            branch: "feature".into(),
        },
        Msg::ForcePush { repo_id },
        Msg::MergeRef {
            repo_id,
            reference: "feature".into(),
        },
        Msg::SquashRef {
            repo_id,
            reference: "feature".into(),
        },
        Msg::ForcePushWithLease {
            repo_id,
            lease: test_force_push_lease(),
        },
        Msg::PushWithTags {
            repo_id,
            request: gitcomet_core::tag_push::TagPushRequest {
                mode: gitcomet_core::tag_push::TagPushMode::All,
                remote: "origin".into(),
                branch: "adjusted/main(unlocked)".into(),
                local_branch: "adjusted/main(unlocked)".into(),
                head: CommitId("2222222222222222222222222222222222222222".into()),
                set_upstream: true,
            },
        },
    ] {
        let label = format!("{msg:?}");
        state.repos[0].feedback.last_error = None;
        let effects = reduce(&mut repos, &id_alloc, &mut state, msg);
        assert!(effects.is_empty(), "{label} must not run git: {effects:?}");
        assert!(
            state.repos[0]
                .feedback
                .last_error
                .as_deref()
                .is_some_and(|error| error.contains("adjusted branch")),
            "{label}: {:?}",
            state.repos[0].feedback.last_error
        );
        assert_eq!(state.repos[0].push_in_flight, 0, "{label}");
        assert_eq!(state.repos[0].pull_in_flight, 0, "{label}");
    }

    // Opting out restores plain Git for all of them.
    state.large_file_settings.annex_pull_push = false;
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::PushSetUpstream {
            repo_id,
            remote: "origin".into(),
            branch: "adjusted/main(unlocked)".into(),
        },
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::PushSetUpstream { .. })),
        "{effects:?}"
    );
}

/// Without git-annex, the takeover cannot run, and a plain merge into the
/// adjusted branch is exactly what it prevents: refuse with a reason.
#[test]
fn adjusted_branch_pull_without_git_annex_is_refused() {
    let (mut repos, id_alloc, mut state, repo_id) = large_file_fixture();
    annex_repo_on(&mut state, "adjusted/main(unlocked)");
    state.large_file_tools.git_annex =
        gitcomet_core::large_file_tools::ToolAvailability::NotFound {
            detail: "Git cannot run `git annex`.".into(),
        };
    for msg in [
        Msg::Pull {
            repo_id,
            mode: PullMode::Default,
        },
        Msg::Push { repo_id },
    ] {
        let effects = reduce(&mut repos, &id_alloc, &mut state, msg);
        assert!(effects.is_empty(), "{effects:?}");
        assert!(
            state.repos[0]
                .feedback
                .last_error
                .as_deref()
                .is_some_and(|error| error.contains("git-annex")),
            "{:?}",
            state.repos[0].feedback.last_error
        );
    }
}

/// Support facts come from config, attributes, HEAD and git-annex; commands
/// that change none of them must not re-run its two git-annex probes.
#[test]
fn only_commands_that_can_change_support_reload_it() {
    use gitcomet_core::large_files::LargeFileCommand;
    let (mut repos, id_alloc, mut state, repo_id) = large_file_fixture();
    let mut reloads = |command: RepoCommandKind| {
        // Settle any load a previous command started.
        state.repos[0].loads_in_flight = Default::default();
        let effects = reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command,
                result: Ok(CommandOutput::default()),
            }),
        );
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadLargeFileSupport { .. }))
    };
    for quiet in [
        RepoCommandKind::StageHunk,
        RepoCommandKind::UnstageHunk,
        RepoCommandKind::Push,
        RepoCommandKind::CreateTag {
            name: "v1".into(),
            target: "HEAD".into(),
            message: None,
            annotated: false,
        },
        RepoCommandKind::FetchAll,
        RepoCommandKind::LargeFile {
            command: LargeFileCommand::LfsLock {
                paths: vec!["a.bin".into()],
            },
        },
        RepoCommandKind::LargeFile {
            command: LargeFileCommand::AnnexGet {
                paths: vec!["a.bin".into()],
                from: None,
            },
        },
        RepoCommandKind::LargeFile {
            command: LargeFileCommand::AnnexDrop {
                paths: vec!["a.bin".into()],
                from: None,
                force: false,
            },
        },
        RepoCommandKind::LargeFile {
            command: LargeFileCommand::AnnexLock {
                paths: vec!["a.bin".into()],
            },
        },
    ] {
        assert!(!reloads(quiet.clone()), "{quiet:?}");
    }
    for loud in [
        RepoCommandKind::LargeFile {
            command: LargeFileCommand::LfsPull {
                paths: vec!["a.bin".into()],
            },
        },
        RepoCommandKind::LargeFile {
            command: LargeFileCommand::AnnexInit,
        },
        RepoCommandKind::Pull {
            mode: PullMode::Default,
        },
        RepoCommandKind::AddRemote {
            name: "backup".into(),
            url: "/tmp/b".into(),
        },
    ] {
        assert!(reloads(loud.clone()), "{loud:?}");
    }
}

#[test]
fn annex_network_commands_release_busy_state_after_failure_or_cancellation() {
    use gitcomet_core::large_files::LargeFileCommand as C;
    for command in [
        C::AnnexPull { content: false },
        C::AnnexPush { content: false },
        C::AnnexSync { content: true },
    ] {
        for kind in [
            gitcomet_core::error::ErrorKind::Cancelled,
            gitcomet_core::error::ErrorKind::Backend("transfer failed".into()),
        ] {
            let (mut repos, id_alloc, mut state, repo_id) = large_file_fixture();
            let message = || Msg::RunLargeFileCommand {
                repo_id,
                command: command.clone(),
            };
            assert_eq!(
                reduce(&mut repos, &id_alloc, &mut state, message()).len(),
                1
            );
            assert_eq!(
                state.repos[0].pull_in_flight,
                u32::from(!matches!(command, C::AnnexPush { .. }))
            );
            assert_eq!(
                state.repos[0].worktree_pull_in_flight,
                state.repos[0].pull_in_flight
            );
            assert_eq!(
                state.repos[0].push_in_flight,
                u32::from(!matches!(command, C::AnnexPull { .. }))
            );
            assert_eq!(state.repos[0].local_actions_in_flight, 1);
            reduce(
                &mut repos,
                &id_alloc,
                &mut state,
                Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
                    repo_id,
                    command: RepoCommandKind::LargeFile {
                        command: command.clone(),
                    },
                    result: Err(gitcomet_core::error::Error::new(kind)),
                }),
            );
            let repo = &state.repos[0];
            assert_eq!(
                (
                    repo.pull_in_flight,
                    repo.worktree_pull_in_flight,
                    repo.push_in_flight,
                    repo.local_actions_in_flight
                ),
                (0, 0, 0, 0)
            );
            assert_eq!(
                reduce(&mut repos, &id_alloc, &mut state, message()).len(),
                1,
                "retry starts after busy state is released"
            );
        }
    }
}

#[test]
fn whereis_caches_both_displayed_keys_and_ignores_superseded_replies() {
    let (mut repos, id_alloc, mut state, repo_id) = large_file_fixture();
    let load = |keys: &[&str]| Msg::LoadAnnexWhereis {
        repo_id,
        keys: keys.iter().map(|key| key.to_string()).collect(),
    };
    let loaded = |key: &str| {
        Msg::Internal(crate::msg::InternalMsg::AnnexWhereisLoaded {
            repo_id,
            key: key.into(),
            result: Ok(gitcomet_core::large_files::AnnexWhereis {
                key: key.into(),
                ..Default::default()
            }),
        })
    };
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        load(&["old", "new", "old"]),
    );
    assert_eq!(
        effects
            .iter()
            .filter(|e| matches!(e, Effect::LoadAnnexWhereis { .. }))
            .count(),
        2
    );
    assert!(
        reduce(&mut repos, &id_alloc, &mut state, load(&["old", "new"])).is_empty(),
        "in-flight lookups coalesce"
    );
    reduce(&mut repos, &id_alloc, &mut state, loaded("new"));
    reduce(&mut repos, &id_alloc, &mut state, loaded("old"));
    for key in ["old", "new"] {
        assert!(
            matches!(state.repos[0].annex_whereis_for(key), Some(Loadable::Ready(whereis)) if whereis.key == key)
        );
    }
    let effects = reduce(&mut repos, &id_alloc, &mut state, load(&["new", "latest"]));
    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, Effect::LoadAnnexWhereis { .. }))
            .count(),
        2,
        "an explicit lookup refreshes locations changed by external tools"
    );
    reduce(&mut repos, &id_alloc, &mut state, loaded("old"));
    assert!(state.repos[0].annex_whereis_for("old").is_none());
    assert!(matches!(
        state.repos[0].annex_whereis_for("latest"),
        Some(Loadable::Loading)
    ));
    use gitcomet_core::large_files::{AnnexTrust, LargeFileCommand};
    for command in [
        LargeFileCommand::AnnexGetKeys {
            keys: vec!["new".into()],
        },
        LargeFileCommand::AnnexTrust {
            repository: "backup".into(),
            trust: AnnexTrust::Trusted,
        },
        LargeFileCommand::AnnexDescribe {
            repository: "backup".into(),
            description: "archive".into(),
        },
    ] {
        reduce(&mut repos, &id_alloc, &mut state, loaded("new"));
        reduce(&mut repos, &id_alloc, &mut state, loaded("latest"));
        let effects = reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command: RepoCommandKind::LargeFile { command },
                result: Ok(CommandOutput::default()),
            }),
        );
        assert_eq!(
            effects
                .iter()
                .filter(|e| matches!(e, Effect::LoadAnnexWhereis { .. }))
                .count(),
            2,
            "content transfers, trust and descriptions refresh both displayed versions"
        );
    }
}

#[test]
fn unused_listing_loads_on_request_and_reloads_after_content_moves() {
    use gitcomet_core::large_files::LargeFileCommand;
    let (mut repos, id_alloc, mut state, repo_id) = large_file_fixture();
    let finished = |command| {
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::LargeFile { command },
            result: Ok(CommandOutput::default()),
        })
    };
    let reloads = |effects: &[Effect]| {
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadAnnexUnused { .. }))
    };
    // Never shown: nothing to keep fresh.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        finished(LargeFileCommand::AnnexDropUnused {
            unused: Default::default(),
            force: false,
        }),
    );
    assert!(!reloads(&effects), "{effects:?}");

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadAnnexUnused { repo_id },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadAnnexUnused { .. }]
    ));
    assert!(matches!(state.repos[0].annex_unused, Loadable::Loading));
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::AnnexUnusedLoaded {
            repo_id,
            result: Ok(gitcomet_core::large_files::AnnexUnused {
                entries: vec![gitcomet_core::large_files::AnnexUnusedEntry {
                    number: 1,
                    key: "SHA256E-s4--old.bin".into(),
                    kind: gitcomet_core::large_files::AnnexUnusedKind::Unused,
                }],
            }),
        }),
    );
    assert!(matches!(
        &state.repos[0].annex_unused,
        Loadable::Ready(unused) if unused.entries.len() == 1
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        finished(LargeFileCommand::AnnexDropUnused {
            unused: Default::default(),
            force: false,
        }),
    );
    assert!(reloads(&effects), "a shown listing reloads: {effects:?}");
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        finished(LargeFileCommand::AnnexDescribe {
            repository: "here".into(),
            description: "x".into(),
        }),
    );
    assert!(!reloads(&effects), "no content moved: {effects:?}");
}

/// Every `git annex unused` run rewrites the numbering files `dropunused`
/// reads, so scans never overlap: a request during one replays after it, and
/// the older result is not shown in the meantime.
#[test]
fn unused_scans_never_overlap() {
    let (mut repos, id_alloc, mut state, repo_id) = large_file_fixture();
    let listing = |key: &str| gitcomet_core::large_files::AnnexUnused {
        entries: vec![gitcomet_core::large_files::AnnexUnusedEntry {
            number: 1,
            key: key.into(),
            kind: gitcomet_core::large_files::AnnexUnusedKind::Unused,
        }],
    };
    macro_rules! send {
        ($msg:expr) => {
            reduce(&mut repos, &id_alloc, &mut state, $msg)
        };
    }
    let scans = |effects: &[Effect]| {
        effects
            .iter()
            .filter(|e| matches!(e, Effect::LoadAnnexUnused { .. }))
            .count()
    };
    assert_eq!(scans(&send!(Msg::LoadAnnexUnused { repo_id })), 1);
    assert_eq!(
        scans(&send!(Msg::LoadAnnexUnused { repo_id })),
        0,
        "one scan at a time"
    );
    let loaded = |key: &str| {
        Msg::Internal(crate::msg::InternalMsg::AnnexUnusedLoaded {
            repo_id,
            result: Ok(listing(key)),
        })
    };
    assert_eq!(scans(&send!(loaded("SHA256E-s1--older"))), 1, "replayed");
    assert!(matches!(state.repos[0].annex_unused, Loadable::Loading));
    assert_eq!(scans(&send!(loaded("SHA256E-s1--newer"))), 0);
    assert!(matches!(
        &state.repos[0].annex_unused,
        Loadable::Ready(unused) if unused.entries[0].key == "SHA256E-s1--newer"
    ));
}
