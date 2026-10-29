use super::*;
use crate::model::{GitLogTagFetchMode, Loadable, RepoIncomingState};
use crate::msg::{FetchMsg, Msg};
use crate::store::AppStore;
use gitcomet_core::domain::HeadTracking;
use gitcomet_core::services::PullMode;

fn incoming_count(state: &crate::model::AppState, workdir: &Path) -> Option<usize> {
    let repo = state
        .repos
        .iter()
        .find(|repo| repo.spec.workdir == workdir)?;
    let snapshot = match &repo.incoming {
        RepoIncomingState::Current(snapshot) | RepoIncomingState::Stale(snapshot) => snapshot,
        RepoIncomingState::Unknown => return None,
    };
    snapshot.tracking.divergence().map(|value| value.behind)
}

fn restore(store: &AppStore, active: &Path, inactive: &Path) {
    store.disable_repo_monitors_for_test();
    store.dispatch(Msg::SetGitLogSettings {
        show_history_tags: false,
        tag_fetch_mode: GitLogTagFetchMode::Disabled,
        verify_commit_signatures: false,
    });
    let mut settings = RemoteSettings::default();
    settings.auto_fetch.enabled = true;
    store.dispatch(Msg::SetRemoteSettings(settings));
    store.dispatch(Msg::RestoreSession {
        open_repos: vec![active.to_path_buf(), inactive.to_path_buf()],
        active_repo: Some(active.to_path_buf()),
    });
}

#[test]
fn incoming_tracking_lazy_tabs_reconcile_selection_pull_and_other_window_checkout() {
    let (temp, source, clone, _) = remote_fixture();
    for contents in ["third", "fourth"] {
        std::fs::write(source.join("file"), contents).unwrap();
        git(&source, &["commit", "-qam", contents]);
    }
    git(&clone, &["branch", "other"]);
    let first = git(&clone, &["rev-parse", "HEAD"]);
    let _session = crate::session::push_test_session_file_path_override(Some(
        temp.path().join("session.json"),
    ));
    let backend: Arc<dyn GitBackend> = Arc::new(gitcomet_git_gix::GixBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let (other, other_events) = AppStore::new_test_sharing_preferences(backend, &store);
    restore(&store, &source, &clone);
    restore(&other, &source, &clone);
    for (store, events) in [(&store, &events), (&other, &other_events)] {
        wait_for_store(store, events, |state| {
            incoming_count(state, &clone) == Some(3)
        });
        let state = store.snapshot();
        let repo = state
            .repos
            .iter()
            .find(|repo| repo.spec.workdir == clone)
            .unwrap();
        assert_ne!(state.active_repo, Some(repo.id));
        assert!(matches!(repo.open, Loadable::NotLoaded));
        assert!(matches!(repo.head_branch, Loadable::NotLoaded));
        assert!(matches!(repo.history_state.log, Loadable::NotLoaded));
        assert!(matches!(repo.status, Loadable::NotLoaded));
        assert!(repo.feedback.diagnostics.is_empty());
        assert!(!repo.incoming_is_stale());
    }
    assert_eq!(git(&clone, &["rev-parse", "HEAD"]), first);
    let repo_id = store
        .snapshot()
        .repos
        .iter()
        .find(|repo| repo.spec.workdir == clone)
        .unwrap()
        .id;
    store.dispatch(Msg::SetActiveRepo { repo_id });
    store
        .barrier_for_test()
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    assert_eq!(incoming_count(&store.snapshot(), &clone), Some(3));
    wait_for_store(&store, &events, |state| {
        state
            .repos
            .iter()
            .any(|repo| repo.id == repo_id && matches!(repo.open, Loadable::Ready(())))
            && incoming_count(state, &clone) == Some(3)
    });
    store.dispatch(Msg::Pull {
        repo_id,
        mode: PullMode::FastForwardOnly,
    });
    wait_for_store(&store, &events, |state| {
        incoming_count(state, &clone) == Some(0)
    });
    wait_for_store(&other, &other_events, |state| {
        incoming_count(state, &clone) == Some(0)
    });
    assert_eq!(
        git(&clone, &["rev-parse", "HEAD"]),
        git(&source, &["rev-parse", "HEAD"])
    );
    store.dispatch(Msg::CheckoutBranch {
        repo_id,
        name: "other".into(),
    });
    for (store, events) in [(&store, &events), (&other, &other_events)] {
        wait_for_store(store, events, |state| {
            state
                .repos
                .iter()
                .find(|repo| repo.spec.workdir == clone)
                .is_some_and(|repo| {
                    matches!(&repo.incoming, RepoIncomingState::Current(snapshot)
                    if snapshot.tracking == HeadTracking::Untracked { branch: "other".into() })
                })
        });
    }
    let other_state = other.snapshot();
    let other_repo = other_state
        .repos
        .iter()
        .find(|repo| repo.spec.workdir == clone)
        .unwrap();
    assert!(matches!(other_repo.open, Loadable::NotLoaded));
    assert!(matches!(other_repo.history_state.log, Loadable::NotLoaded));
    assert!(matches!(other_repo.status, Loadable::NotLoaded));
    drop(other_state);
    for store in [&store, &other] {
        store
            .shutdown_for_test()
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
    }
}

#[test]
fn incoming_tracking_broadcast_includes_linked_tabs_in_the_initiating_store() {
    use std::sync::Mutex;

    let coordinator = AutoFetchCoordinator::new();
    let received = Arc::new(Mutex::new(Vec::new()));
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    for (store_id, registrations) in [
        (
            1,
            vec![(1, "/repo", "/repo/.git"), (2, "/linked", "/repo/.git")],
        ),
        (2, vec![(3, "/repo", "/repo/.git")]),
        (3, vec![(4, "/other", "/other/.git")]),
    ] {
        let received = Arc::clone(&received);
        coordinator.sync(
            store_id,
            registrations
                .into_iter()
                .map(|(id, workdir, common)| Registration {
                    repo_id: RepoId(id),
                    lifetime: RepoLifetime::allocate(),
                    workdir: PathBuf::from(workdir),
                    common_dir: Some(PathBuf::from(common)),
                    repository: None,
                    active: false,
                    needs_refresh: true,
                })
                .collect(),
            RemoteSettings::default(),
            &backend,
            Arc::new(move |event| {
                if let Event::Refresh { repo_id, .. } = event {
                    received.lock().unwrap().push(repo_id.0);
                }
            }),
        );
    }
    coordinator.refresh_other_subscribers(
        1,
        RepoId(1),
        Path::new("/repo"),
        Path::new("/repo/.git"),
    );
    let mut received = received.lock().unwrap().clone();
    received.sort_unstable();
    assert_eq!(received, vec![2, 3]);
}

#[test]
fn incoming_tracking_manual_fetch_failure_stays_stale_after_cached_reference_reads() {
    let (temp, source, clone, _) = remote_fixture();
    let _session = crate::session::push_test_session_file_path_override(Some(
        temp.path().join("session.json"),
    ));
    let backend: Arc<dyn GitBackend> = Arc::new(gitcomet_git_gix::GixBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let (other, other_events) = AppStore::new_test_sharing_preferences(backend, &store);
    restore(&store, &clone, &source);
    restore(&other, &source, &clone);
    for (store, events) in [(&store, &events), (&other, &other_events)] {
        wait_for_store(store, events, |state| {
            incoming_count(state, &clone) == Some(1)
                && state
                    .repos
                    .iter()
                    .find(|repo| repo.spec.workdir == clone)
                    .is_some_and(|repo| {
                        !repo.incoming_is_stale()
                            && matches!(repo.auto_fetch.status, AutoFetchStatus::Succeeded { .. })
                    })
        });
    }
    // Auto-fetch opens its own handle, so incoming can be current before the
    // tab opens (a slow session write delays it); a manual fetch sent then
    // fails without running git.
    wait_for_store(&store, &events, |state| {
        state.repos.iter().any(|repo| {
            repo.spec.workdir == clone
                && matches!(repo.open, Loadable::Ready(()))
                && !repo
                    .loads_in_flight
                    .is_in_flight(crate::model::LoadKind::UpstreamDivergence)
        })
    });
    let snapshots = [store.snapshot(), other.snapshot()];
    let originals = snapshots
        .iter()
        .map(|state| {
            let repo = state
                .repos
                .iter()
                .find(|repo| repo.spec.workdir == clone)
                .unwrap();
            (
                repo.id,
                repo.upstream_divergence_rev,
                repo.auto_fetch.last_success_at,
            )
        })
        .collect::<Vec<_>>();
    let missing = temp.path().join("remote-unavailable");
    git(
        &clone,
        &["remote", "set-url", "origin", missing.to_str().unwrap()],
    );
    store.dispatch(Msg::Fetch(FetchMsg::All {
        repo_id: originals[0].0,
    }));
    wait_for_store(&store, &events, |state| {
        state
            .repos
            .iter()
            .find(|repo| repo.spec.workdir == clone)
            .is_some_and(|repo| {
                repo.feedback.command_log.iter().rev().any(|entry| {
                    entry.source == crate::model::CommandSource::User
                        && !entry.ok
                        && entry.command.contains("fetch")
                })
            })
    });
    for (index, (store, events)) in [(&store, &events), (&other, &other_events)]
        .into_iter()
        .enumerate()
    {
        store
            .barrier_for_test()
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        wait_for_store(store, events, |state| {
            state
                .repos
                .iter()
                .find(|repo| repo.spec.workdir == clone)
                .is_some_and(|repo| {
                    repo.upstream_divergence_rev > originals[index].1
                        && !repo
                            .loads_in_flight
                            .is_in_flight(crate::model::LoadKind::UpstreamDivergence)
                })
        });
        let state = store.snapshot();
        let repo = state
            .repos
            .iter()
            .find(|repo| repo.spec.workdir == clone)
            .unwrap();
        assert_eq!(incoming_count(&state, &clone), Some(1));
        assert!(
            repo.incoming_is_stale(),
            "failed remote check must outlive a successful cached read"
        );
        assert_eq!(repo.auto_fetch.last_success_at, originals[index].2);
        if index == 1 {
            assert!(matches!(repo.open, Loadable::NotLoaded));
            assert!(matches!(repo.history_state.log, Loadable::NotLoaded));
            assert!(matches!(repo.status, Loadable::NotLoaded));
            assert!(repo.feedback.diagnostics.is_empty());
        }
    }
    git(
        &clone,
        &["remote", "set-url", "origin", source.to_str().unwrap()],
    );
    store.dispatch(Msg::Fetch(FetchMsg::All {
        repo_id: originals[0].0,
    }));
    for (index, (store, events)) in [(&store, &events), (&other, &other_events)]
        .into_iter()
        .enumerate()
    {
        wait_for_store(store, events, |state| {
            incoming_count(state, &clone) == Some(1)
                && state
                    .repos
                    .iter()
                    .find(|repo| repo.spec.workdir == clone)
                    .is_some_and(|repo| {
                        !repo.incoming_is_stale()
                            && repo.auto_fetch.last_success_at != originals[index].2
                            && matches!(repo.auto_fetch.status, AutoFetchStatus::Succeeded { .. })
                    })
        });
    }
    for store in [&store, &other] {
        store
            .shutdown_for_test()
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
    }
}

#[test]
fn incoming_tracking_manual_failure_preserves_deadline_and_rejects_older_auto_result() {
    let coordinator = AutoFetchCoordinator::new();
    register(&coordinator, 1, "/repo", "/repo/.git");
    enable(&coordinator);
    let path = Path::new("/repo");
    let previous = UNIX_EPOCH + Duration::from_secs(100);
    let success = Ok(CommandOutput::empty_success("git fetch --all"));
    coordinator.manual_fetch_finished(path, previous, &success);
    let started = previous + AutoFetchInterval::default().duration();
    let job = next_job(
        &mut coordinator.inner.state.lock().unwrap(),
        coordinator.admission(),
        started,
    )
    .unwrap();
    let failed_at = started + Duration::from_secs(5);
    coordinator.manual_fetch_finished(
        path,
        failed_at,
        &Err(Error::new(ErrorKind::Backend("remote unavailable".into()))),
    );
    coordinator.manual_fetch_finished(
        path,
        failed_at + Duration::from_secs(1),
        &Err(Error::new(ErrorKind::Cancelled)),
    );
    let mut state = coordinator.inner.state.lock().unwrap();
    finish(
        &mut state,
        &job,
        Completion::Finished {
            common_dir: Some(PathBuf::from("/repo/.git")),
            key: None,
            remotes: Vec::new(),
            skipped: Vec::new(),
            result: Err(Error::new(ErrorKind::Cancelled)),
            refs_changed: true,
            regrouped: Vec::new(),
            tracking: Vec::new(),
        },
        failed_at + Duration::from_secs(2),
    );
    let group = group_of(&state, path);
    assert_eq!(group.schedule.completed, Some(previous));
    assert_eq!(group.snapshot.last_success_at, Some(previous));
    assert!(matches!(group.snapshot.status, AutoFetchStatus::Failed { at, .. } if at == failed_at));
    drop(state);
    let recovered = failed_at + Duration::from_secs(3);
    coordinator.manual_fetch_finished(path, recovered, &success);
    let state = coordinator.inner.state.lock().unwrap();
    let group = group_of(&state, path);
    assert_eq!(group.schedule.completed, Some(recovered));
    assert_eq!(group.snapshot.last_success_at, Some(recovered));
    assert!(matches!(group.snapshot.status, AutoFetchStatus::Succeeded { at } if at == recovered));
}

#[test]
fn incoming_tracking_rewording_a_published_tip_updates_other_windows() {
    let (temp, _source, clone, _) = remote_fixture();
    git(&clone, &["pull", "-q", "--ff-only"]);
    let head = git(&clone, &["rev-parse", "HEAD"]);
    let _session = crate::session::push_test_session_file_path_override(Some(
        temp.path().join("session.json"),
    ));
    let backend: Arc<dyn GitBackend> = Arc::new(gitcomet_git_gix::GixBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let (other, other_events) = AppStore::new_test_sharing_preferences(backend, &store);
    let source = clone.parent().unwrap().join("source");
    restore(&store, &clone, &source);
    restore(&other, &source, &clone);
    for (store, events) in [(&store, &events), (&other, &other_events)] {
        wait_for_store(store, events, |state| {
            incoming_count(state, &clone) == Some(0)
        });
    }
    wait_for_store(&store, &events, |state| {
        state
            .repos
            .iter()
            .any(|repo| repo.spec.workdir == clone && matches!(repo.open, Loadable::Ready(())))
    });
    let repo_id = store
        .snapshot()
        .repos
        .iter()
        .find(|repo| repo.spec.workdir == clone)
        .unwrap()
        .id;
    store.dispatch(Msg::RewordCommit {
        repo_id,
        request_id: crate::model::RewordRequestId::new(),
        commit_id: gitcomet_core::domain::CommitId(head.clone().into()),
        message: "reworded".into(),
    });
    wait_for_store(&store, &events, |state| {
        incoming_count(state, &clone) == Some(1)
    });
    assert_ne!(git(&clone, &["rev-parse", "HEAD"]), head);
    // The other window's tab is not selected: only a broadcast reloads it.
    wait_for_store(&other, &other_events, |state| {
        incoming_count(state, &clone) == Some(1)
    });
    for store in [&store, &other] {
        store
            .shutdown_for_test()
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
    }
}

/// Fails opening `path` while `failures` lasts, as a briefly unreadable disk.
struct FlakyOpens {
    path: PathBuf,
    failures: std::sync::atomic::AtomicUsize,
}

impl FlakyOpens {
    fn check(&self, path: &Path) -> Result<()> {
        use std::sync::atomic::Ordering;
        if canonicalize_or_original(path.to_path_buf()) == self.path
            && self
                .failures
                .try_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                    left.checked_sub(1)
                })
                .is_ok()
        {
            return Err(Error::new(ErrorKind::Backend(
                "disk briefly unreadable".into(),
            )));
        }
        Ok(())
    }
}

impl GitBackend for FlakyOpens {
    fn open(&self, path: &Path) -> Result<Arc<dyn GitRepository>> {
        self.check(path)?;
        gitcomet_git_gix::GixBackend.open(path)
    }

    fn open_cancellable(
        &self,
        path: &Path,
        cancellation: &CancellationToken,
    ) -> Result<Arc<dyn GitRepository>> {
        self.check(path)?;
        gitcomet_git_gix::GixBackend.open_cancellable(path, cancellation)
    }
}

#[test]
fn incoming_tracking_read_that_fails_is_retried_by_the_next_unchanged_fetch() {
    use std::sync::atomic::Ordering;

    let (temp, source, clone, _) = remote_fixture();
    let _session = crate::session::push_test_session_file_path_override(Some(
        temp.path().join("session.json"),
    ));
    let backend = Arc::new(FlakyOpens {
        path: clone.clone(),
        failures: Default::default(),
    });
    let (store, events) = AppStore::new_test(Arc::clone(&backend) as Arc<dyn GitBackend>);
    let coordinator = Arc::clone(&store.msg_tx.auto_fetch);
    // The test runs every fetch itself; this hold keeps the worker idle.
    const HOLD: u64 = u64::MAX;
    coordinator.set_sign_in_open(HOLD, true);
    restore(&store, &source, &clone);
    wait_for_store(&store, &events, |_| {
        coordinator.inner.state.lock().unwrap().entries.len() == 2
    });
    let run_due = |now: SystemTime, fail_read: bool| {
        loop {
            let job = {
                let mut state = coordinator.inner.state.lock().unwrap();
                state.signing_in.remove(&HOLD);
                let job = next_job(&mut state, coordinator.admission(), now);
                state.signing_in.insert(HOLD);
                job
            };
            let Some(job) = job else {
                return;
            };
            let completion = execute(&job, &coordinator.inner);
            if fail_read && job.workdir == clone {
                // The tracking read this fetch triggers fails once.
                backend.failures.store(1, Ordering::SeqCst);
            }
            finish(
                &mut coordinator.inner.state.lock().unwrap(),
                &job,
                completion,
                now,
            );
        }
    };
    let settled = |state: &crate::model::AppState| {
        state.repos.iter().any(|repo| {
            repo.spec.workdir == clone
                && matches!(repo.auto_fetch.status, AutoFetchStatus::Failed { .. })
                && !repo
                    .loads_in_flight
                    .is_in_flight(crate::model::LoadKind::UpstreamDivergence)
        })
    };
    let started = SystemTime::now();
    run_due(started, true);
    wait_for_store(&store, &events, settled);
    assert_eq!(
        backend.failures.load(Ordering::SeqCst),
        0,
        "the read failed"
    );
    assert_eq!(incoming_count(&store.snapshot(), &clone), None);
    run_due(started + AutoFetchInterval::default().duration(), false);
    wait_for_store(&store, &events, |state| {
        incoming_count(state, &clone) == Some(1)
            && state
                .repos
                .iter()
                .any(|repo| repo.spec.workdir == clone && !repo.incoming_is_stale())
    });
    coordinator.set_sign_in_open(HOLD, false);
    store
        .shutdown_for_test()
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
}
