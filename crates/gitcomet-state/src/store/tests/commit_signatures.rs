use super::*;
use gitcomet_core::domain::{CommitSignature, SignatureFormat, SignatureFormats, SignatureStatus};
use gitcomet_core::signing_tools::{SigningToolAvailability, SigningToolsState};

fn id(n: usize) -> CommitId {
    CommitId(format!("{n:040x}").into())
}
fn good_signature() -> CommitSignature {
    CommitSignature {
        status: SignatureStatus::Good,
        format: SignatureFormat::OpenPgp,
        signer: Some(Arc::from("Ada")),
        key_id: Some(Arc::from("ABCD")),
    }
}
fn checked_tools() -> SigningToolsState {
    let mut tools = SigningToolsState::default();
    tools.gpg.availability = SigningToolAvailability::Available {
        version: Some("test gpg".into()),
    };
    tools.ssh_keygen.availability = SigningToolAvailability::Unknown;
    tools
}

#[test]
fn configuration_changes_invalidate_verdicts_but_ordinary_refreshes_do_not() {
    let mut fixture = Fixture::new();
    let running = fixture.targets([id(1)]).unwrap();
    fixture.finish(&running, true);
    fixture.send(Msg::RepoExternallyChanged {
        repo_id: RepoId(1),
        change: crate::msg::RepoExternalChange::all(),
    });
    assert!(
        fixture.state.repos[0]
            .history_state
            .commit_signatures
            .contains_key(&id(1))
    );
    fixture.send(Msg::RepoExternallyChanged {
        repo_id: RepoId(1),
        change: crate::msg::RepoExternalChange {
            verification_context: true,
            ..crate::msg::RepoExternalChange::all()
        },
    });
    assert!(
        fixture.state.repos[0]
            .history_state
            .commit_signatures
            .is_empty()
    );
    assert!(running.cancellation.is_cancelled());
    assert!(fixture.state.signature_verification_formats().is_empty());
    assert!(fixture.targets([id(1)]).is_none());
    fixture.send(Msg::SetSigningToolsState(checked_tools()));
    assert_eq!(&*fixture.targets([id(1)]).unwrap().ids, &[id(1)]);
}
#[derive(Clone)]
struct Batch {
    epoch: u64,
    batch: u64,
    ids: Arc<[CommitId]>,
    cancellation: CancellationToken,
    formats: SignatureFormats,
}
fn batch(effects: Vec<Effect>) -> Option<Batch> {
    let mut batches = effects.into_iter().filter_map(|effect| match effect {
        Effect::VerifyCommitSignatures {
            epoch,
            batch,
            commit_ids,
            cancellation,
            formats,
            ..
        } => Some(Batch {
            epoch,
            batch,
            ids: commit_ids,
            cancellation,
            formats,
        }),
        _ => None,
    });
    let first = batches.next();
    assert!(batches.next().is_none(), "only one batch at a time");
    first
}
struct Fixture {
    state: AppState,
    repos: FxHashMap<RepoId, Arc<dyn GitRepository>>,
}
impl Fixture {
    fn new() -> Self {
        let mut state = AppState::test_default();
        state.git_log_settings.verify_commit_signatures = true;
        state.signing_tools = checked_tools();
        state.repos.push(RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        ));
        state.active_repo = Some(RepoId(1));
        Self {
            state,
            repos: FxHashMap::default(),
        }
    }
    fn send(&mut self, msg: Msg) -> Option<Batch> {
        batch(reduce(
            &mut self.repos,
            &AtomicU64::new(2),
            &mut self.state,
            msg,
        ))
    }
    fn targets(&mut self, ids: impl IntoIterator<Item = CommitId>) -> Option<Batch> {
        self.send(Msg::SetCommitSignatureTargets {
            repo_id: RepoId(1),
            epoch: self.state.repos[0].history_state.commit_signatures_epoch,
            commit_ids: ids.into_iter().collect(),
        })
    }
    fn finish(&mut self, running: &Batch, signed: bool) -> Option<Batch> {
        self.send(Msg::Internal(
            crate::msg::InternalMsg::CommitSignaturesVerified {
                repo_id: RepoId(1),
                epoch: running.epoch,
                batch: running.batch,
                result: Ok(if signed {
                    running
                        .ids
                        .iter()
                        .cloned()
                        .map(|id| (id, good_signature()))
                        .collect()
                } else {
                    Vec::new()
                }),
            },
        ))
    }
    fn setting(&mut self, enabled: bool) -> Option<Batch> {
        self.send(Msg::SetGitLogSettings {
            show_history_tags: true,
            tag_fetch_mode: self.state.git_log_settings.tag_fetch_mode,
            verify_commit_signatures: enabled,
        })
    }
    fn log_reply(
        &mut self,
        result: Result<gitcomet_core::services::HistoryReadResult>,
    ) -> Option<Batch> {
        let seq = self.state.repos[0]
            .loads_in_flight
            .request_log(crate::model::PendingLogLoad {
                scope: LogScope::AllBranches,
                author: None,
                solo: Default::default(),
                limit: 200,
                cursor: None,
            })
            .unwrap();
        self.send(Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            seq,
            scope: LogScope::AllBranches,
            cursor: None,
            result,
        }))
    }
}

#[test]
fn defaults_off_and_disabled_targets_allocate_no_scheduler_state() {
    assert!(!crate::model::GitLogSettings::default().verify_commit_signatures);
    let mut f = Fixture::new();
    f.setting(false);
    let history = &f.state.repos[0].history_state;
    let attempts = history.commit_signatures_requested.clone();
    let visible = history.commit_signatures_visible.clone();
    let epoch = history.commit_signatures_epoch;
    for _ in 0..10 {
        assert!(f.targets((0..200).map(id)).is_none());
    }
    f.state.repos.push(RepoState::new_opening(
        RepoId(2),
        RepoSpec {
            workdir: PathBuf::from("/tmp/other"),
        },
    ));
    assert!(f.send(Msg::SetActiveRepo { repo_id: RepoId(2) }).is_none());
    assert!(f.send(Msg::SetActiveRepo { repo_id: RepoId(1) }).is_none());
    let h = &f.state.repos[0].history_state;
    assert!(Arc::ptr_eq(&attempts, &h.commit_signatures_requested));
    assert!(Arc::ptr_eq(&visible, &h.commit_signatures_visible));
    assert!(h.commit_signatures_queue.is_empty());
    assert_eq!(epoch, h.commit_signatures_epoch);
}

#[test]
fn visible_demand_is_batched_and_unsigned_attempts_are_memoized() {
    let mut f = Fixture::new();
    let mut current = f.targets((0..40).map(id));
    let mut seen = Vec::new();
    while let Some(running) = current {
        assert!(running.ids.len() <= 16);
        seen.extend(running.ids.iter().cloned());
        current = f.finish(&running, false);
    }
    assert_eq!(seen, (0..40).map(id).collect::<Vec<_>>());
    assert!(f.targets((0..40).map(id)).is_none());
}

#[test]
fn scrolling_replaces_pending_work_and_selected_commit_goes_first() {
    let mut f = Fixture::new();
    let first = f.targets((0..60).map(id)).unwrap();
    assert!(f.targets((100..140).map(id)).is_none());
    assert!(
        f.send(Msg::SelectCommit {
            repo_id: RepoId(1),
            commit_id: id(999)
        })
        .is_none()
    );
    let next = f.finish(&first, false).unwrap();
    assert_eq!(next.ids[0], id(999));
    assert_eq!(next.ids[1], id(100));
    assert!(!first.cancellation.is_cancelled());
    assert!(
        !f.state.repos[0]
            .history_state
            .commit_signatures_requested
            .contains(&id(20))
    );
}

#[test]
fn selection_verifies_immediately_without_waiting_for_details_or_viewport() {
    let mut f = Fixture::new();
    let running = f
        .send(Msg::SelectCommit {
            repo_id: RepoId(1),
            commit_id: id(999),
        })
        .unwrap();
    assert_eq!(running.ids.as_ref(), &[id(999)]);
    assert!(f.finish(&running, true).is_none());
    assert!(
        f.send(Msg::SelectCommit {
            repo_id: RepoId(1),
            commit_id: id(999)
        })
        .is_none()
    );
}

#[test]
fn stale_batch_cannot_release_a_newer_batch_or_publish_badges() {
    let mut f = Fixture::new();
    let first = f.targets((0..40).map(id)).unwrap();
    let second = f.finish(&first, false).unwrap();
    assert!(f.finish(&first, true).is_none());
    assert!(f.state.repos[0].history_state.commit_signatures_in_flight);
    assert!(f.state.repos[0].history_state.commit_signatures.is_empty());
    assert!(f.finish(&second, false).is_some());
}

#[test]
fn disable_cancels_clears_badges_and_invalidates_late_replies_even_after_reenable() {
    let mut f = Fixture::new();
    let first = f.targets((0..40).map(id)).unwrap();
    let second = f.finish(&first, true).unwrap();
    assert_eq!(f.state.repos[0].history_state.commit_signatures.len(), 16);
    f.setting(false);
    assert!(second.cancellation.is_cancelled());
    assert!(f.state.repos[0].history_state.commit_signatures.is_empty());
    f.setting(true);
    f.send(Msg::SetSigningToolsState(checked_tools()));
    let newest = f.targets([id(100)]).unwrap();
    assert!(f.finish(&second, true).is_none());
    assert!(f.state.repos[0].history_state.commit_signatures_in_flight);
    f.finish(&newest, true);
    assert_eq!(f.state.repos[0].history_state.commit_signatures.len(), 1);
}

#[test]
fn inactive_tab_finishes_current_batch_but_starts_no_more_work() {
    let mut f = Fixture::new();
    let first = f.targets((0..40).map(id)).unwrap();
    f.state.active_repo = None;
    assert!(f.targets([id(100)]).is_none());
    assert!(f.finish(&first, false).is_none());
    assert!(!f.state.repos[0].history_state.commit_signatures_in_flight);
    f.state.active_repo = Some(RepoId(1));
    assert_eq!(f.targets([id(100)]).unwrap().ids.as_ref(), &[id(100)]);
}

#[test]
fn closing_repositories_cancels_signature_work() {
    for bulk in [false, true] {
        let mut f = Fixture::new();
        let running = f.targets([id(1)]).unwrap();
        f.send(if bulk {
            Msg::CloseRepos {
                repo_ids: vec![RepoId(1)],
                activate_after: None,
            }
        } else {
            Msg::CloseRepo { repo_id: RepoId(1) }
        });
        assert!(running.cancellation.is_cancelled());
        assert!(f.finish(&running, true).is_none());
    }
}

#[test]
fn attempts_and_verdicts_are_bounded_and_evicted_commits_can_be_revisited() {
    let mut f = Fixture::new();
    for start in (0..5008).step_by(16) {
        let running = f.targets((start..start + 16).map(id)).unwrap();
        f.finish(&running, true);
    }
    let h = &f.state.repos[0].history_state;
    assert_eq!(h.commit_signatures_requested.len(), 4096);
    assert_eq!(h.commit_signatures_attempt_order.len(), 4096);
    assert_eq!(h.commit_signatures.len(), 4096);
    assert!(f.targets([id(0)]).is_some());
}

#[test]
fn unprobed_tools_hold_demand_until_discovery_then_ui_republishes_for_new_epoch() {
    let mut f = Fixture::new();
    f.state.signing_tools = SigningToolsState::default();
    assert!(f.targets([id(1)]).is_none());
    assert!(
        f.state.repos[0]
            .history_state
            .commit_signatures_requested
            .is_empty()
    );
    assert!(f.send(Msg::SetSigningToolsState(checked_tools())).is_none());
    assert!(f.targets([id(1)]).is_some());
}

#[test]
fn full_tool_identity_changes_invalidate_trust_even_with_same_formats() {
    let mut f = Fixture::new();
    let running = f.targets([id(1)]).unwrap();
    f.finish(&running, true);
    assert!(f.send(Msg::SetSigningToolsState(checked_tools())).is_none());
    assert!(!running.cancellation.is_cancelled());
    let mut tools = checked_tools();
    tools.gpg.program = "another-gpg".into();
    f.send(Msg::SetSigningToolsState(tools));
    assert!(running.cancellation.is_cancelled());
    assert!(f.state.repos[0].history_state.commit_signatures.is_empty());
    assert!(f.targets([id(1)]).is_some());
}

#[test]
fn only_formats_with_completed_available_or_inconclusive_probes_are_used() {
    let mut f = Fixture::new();
    f.state.signing_tools.gpg.availability = SigningToolAvailability::NotFound {
        detail: "missing".into(),
    };
    assert_eq!(
        f.targets([id(1)]).unwrap().formats,
        SignatureFormats::NONE.with(SignatureFormat::Ssh)
    );
    let mut tools = checked_tools();
    tools.gpg.availability = SigningToolAvailability::NotFound {
        detail: "missing".into(),
    };
    tools.ssh_keygen.availability = tools.gpg.availability.clone();
    f.send(Msg::SetSigningToolsState(tools));
    assert!(f.targets([id(2)]).is_none());
}

#[test]
fn unchanged_and_failed_history_checks_preserve_verification() {
    for result in [
        Ok(gitcomet_core::services::HistoryReadResult::Unchanged),
        Err(Error::new(ErrorKind::Unsupported("test"))),
    ] {
        let mut f = Fixture::new();
        let first = f.targets((0..40).map(id)).unwrap();
        let second = f.finish(&first, true).unwrap();
        let previous = f.state.repos[0].history_state.commit_signatures.clone();
        f.log_reply(result);
        assert!(Arc::ptr_eq(
            &previous,
            &f.state.repos[0].history_state.commit_signatures
        ));
        assert!(!second.cancellation.is_cancelled());
        assert!(f.finish(&second, false).is_some());
    }
}

#[test]
fn replaced_history_invalidates_trust_without_eagerly_verifying_loaded_rows() {
    let mut f = Fixture::new();
    let first = f.targets((0..40).map(id)).unwrap();
    let page = Arc::new(LogPage {
        commits: Vec::new(),
        next_cursor: None,
    });
    assert!(
        f.log_reply(Ok(gitcomet_core::services::HistoryReadResult::Page {
            page,
            snapshot: None
        }))
        .is_none()
    );
    assert!(first.cancellation.is_cancelled());
    assert!(f.finish(&first, true).is_none());
    assert!(f.targets([id(1)]).is_some());
}

#[test]
fn merging_one_badge_preserves_sparse_copy_on_write_and_published_snapshots() {
    let mut f = Fixture::new();
    for start in (0..1024).step_by(16) {
        let running = f.targets((start..start + 16).map(id)).unwrap();
        f.finish(&running, true);
    }
    let running = f.targets([id(2000)]).unwrap();
    let previous = f.state.repos[0].history_state.commit_signatures.clone();
    let signer = previous
        .get(&id(0))
        .unwrap()
        .signer
        .as_ref()
        .unwrap()
        .clone();
    let before = Arc::strong_count(&signer);
    f.finish(&running, true);
    assert!(Arc::strong_count(&signer) - before <= 1);
    assert_eq!(previous.len(), 1024);
    assert_eq!(f.state.repos[0].history_state.commit_signatures.len(), 1025);
}
