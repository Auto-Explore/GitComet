//! Prepare repository work before native window/renderer initialization.
use super::*;
use gitcomet_core::process::{GitExecutableAvailability, GitRuntimeState};
use std::sync::{Mutex, Weak};

enum GitBootstrapState {
    Waiting(Option<DeferredRepoBootstrap>),
    Dispatched(GitRuntimeState),
    Attached,
}

/// The worker can adopt Git and consume an optional restore only until the
/// native window attaches. Queueing the messages under this lock makes that
/// ownership transfer exact even before the store has reduced the messages.
struct GitBootstrapHandoff {
    store: AppStore,
    expected_runtime: GitRuntimeState,
    state: Mutex<GitBootstrapState>,
}

impl GitBootstrapHandoff {
    fn complete(&self, runtime: GitRuntimeState, current_runtime: &GitRuntimeState) {
        let mut state = self.state.lock().unwrap_or_else(|err| err.into_inner());
        if !matches!(*state, GitBootstrapState::Waiting(_))
            || !runtime.is_available()
            || runtime != *current_runtime
            || runtime.preference != self.expected_runtime.preference
            || self.store.snapshot().git_runtime != self.expected_runtime
        {
            return;
        }
        let GitBootstrapState::Waiting(bootstrap) =
            std::mem::replace(&mut *state, GitBootstrapState::Dispatched(runtime.clone()))
        else {
            unreachable!();
        };
        self.store.dispatch(Msg::SetGitRuntimeState(runtime));
        let has_repository_bootstrap = bootstrap.is_some();
        match bootstrap {
            Some(DeferredRepoBootstrap::RestoreSession {
                open_repos,
                active_repo,
            }) => self.store.dispatch(Msg::RestoreSession {
                open_repos,
                active_repo,
            }),
            Some(DeferredRepoBootstrap::OpenRepos(paths)) => {
                for path in paths {
                    self.store.dispatch(Msg::OpenRepo(path));
                }
            }
            None => {}
        }
        drop(state);
        gitcomet_core::startup_trace::mark(if has_repository_bootstrap {
            "repository_bootstrap_dispatched_after_git_probe"
        } else {
            "home_git_runtime_dispatched_after_probe"
        });
    }
}

/// Owns the bootstrap decision until the view attaches. An empty store snapshot
/// does not mean a second restore is needed: the first may still be queued.
pub(crate) struct PreparedWindowBootstrap {
    pub(super) config: GitCometViewConfig,
    pub(super) ui_session: session::UiSession,
    pub(super) ui_preferences: UiPreferences,
    pub(super) workspace_id: Option<session::WorkspaceId>,
    pub(super) persisted_workspace_repo_paths: Vec<std::path::PathBuf>,
    pub(super) persisted_workspace_active_repository: Option<std::path::PathBuf>,
    pub(super) initial_window_placement: Option<session::PortableWindowPlacement>,
    pub(super) workspace_theme_key: Option<String>,
    pub(super) entry_denials: Vec<SharedString>,
    pub(super) startup_repo_bootstrap_pending: bool,
    pub(super) startup_repo_open_failure_revision: u64,
    pub(super) deferred_repo_bootstrap: Option<DeferredRepoBootstrap>,
    pub(super) pending_repo_open_reservations: FxHashMap<std::path::PathBuf, PendingRepoOpen>,
    pub(super) pending_repo_open_active: Option<std::path::PathBuf>,
    git_probe_handoff: Option<Arc<GitBootstrapHandoff>>,
}

impl PreparedWindowBootstrap {
    pub(crate) fn new(store: &AppStore, config: GitCometViewConfig, cx: &mut App) -> Self {
        let mut bootstrap = Self::from_session(store, config, session::load(), cx);
        if !cfg!(test)
            && let Some(handoff) = bootstrap.begin_git_probe_handoff(store)
        {
            cx.background_executor()
                .spawn(async move {
                    // Wait only for discovery already started by the browser.
                    // Missing Git and abandoned probes keep the usual UI recovery.
                    let runtime =
                        gitcomet_core::process::wait_for_git_runtime_probe(Duration::from_secs(10));
                    if let Some(runtime) = runtime
                        && let Some(handoff) = handoff.upgrade()
                    {
                        handoff.complete(runtime, &gitcomet_core::process::current_git_runtime());
                    }
                })
                .detach();
        }
        bootstrap
    }

    fn begin_git_probe_handoff(&mut self, store: &AppStore) -> Option<Weak<GitBootstrapHandoff>> {
        if self.git_probe_handoff.is_some() {
            return None;
        }
        let initial_state = store.snapshot();
        // Home also needs Git before its Open/Clone/Init actions are usable.
        // A store created before discovery finishes would otherwise attach
        // with Checking and wait for a foreground report after its first frame.
        let home_waiting_for_git = self.config.view_mode == GitCometViewMode::Normal
            && initial_state.repos.is_empty()
            && matches!(
                initial_state.git_runtime.availability,
                GitExecutableAvailability::Checking
            );
        if self.deferred_repo_bootstrap.is_none() && !home_waiting_for_git {
            return None;
        }
        let bootstrap = self.deferred_repo_bootstrap.take();
        let handoff = Arc::new(GitBootstrapHandoff {
            store: store.clone(),
            expected_runtime: initial_state.git_runtime.clone(),
            state: Mutex::new(GitBootstrapState::Waiting(bootstrap)),
        });
        let worker = Arc::downgrade(&handoff);
        self.git_probe_handoff = Some(handoff);
        Some(worker)
    }

    pub(super) fn attach_git_probe_handoff(&mut self) -> Option<GitRuntimeState> {
        let handoff = self.git_probe_handoff.take()?;
        let mut state = handoff.state.lock().unwrap_or_else(|err| err.into_inner());
        match std::mem::replace(&mut *state, GitBootstrapState::Attached) {
            GitBootstrapState::Waiting(bootstrap) => {
                self.deferred_repo_bootstrap = bootstrap;
                None
            }
            GitBootstrapState::Dispatched(runtime) => Some(runtime),
            GitBootstrapState::Attached => None,
        }
    }

    fn from_session(
        store: &AppStore,
        mut config: GitCometViewConfig,
        mut ui_session: session::UiSession,
        cx: &mut App,
    ) -> Self {
        let _stage = gitcomet_core::startup_trace::span("repository_bootstrap_prepare");
        let view_mode = config.view_mode;
        let initial_repository_launch_mode = config.initial_repository_launch_mode;
        let workspace = config.workspace.clone();
        let mut initial_path = config.initial_path.take().or_else(|| {
            config
                .focused_mergetool
                .as_ref()
                .map(|cfg| cfg.repo_path.clone())
        });
        let (
            workspace_id,
            persisted_workspace_repo_paths,
            persisted_workspace_active_repository,
            initial_window_placement,
            workspace_theme_key,
        ) = match workspace {
            WorkspaceBootstrap::LegacySession => (None, Vec::new(), None, None, None),
            WorkspaceBootstrap::Empty => {
                ui_session.open_repos.clear();
                ui_session.active_repo = None;
                ui_session.sidebar_width = None;
                ui_session.details_width = None;
                ui_session.sidebar_collapsed = Some(false);
                ui_session.change_tracking_height = None;
                ui_session.untracked_height = None;
                (None, Vec::new(), None, None, None)
            }
            WorkspaceBootstrap::Saved(workspace) => {
                let workspace = *workspace;
                ui_session.open_repos.clone_from(&workspace.repositories);
                ui_session
                    .active_repo
                    .clone_from(&workspace.active_repository);
                ui_session.sidebar_width = workspace.layout.sidebar_width;
                ui_session.details_width = workspace.layout.details_width;
                ui_session.sidebar_collapsed = Some(workspace.layout.sidebar_collapsed);
                ui_session.change_tracking_height = workspace.layout.change_tracking_height;
                ui_session.untracked_height = workspace.layout.untracked_height;
                (
                    Some(workspace.id),
                    workspace.repositories,
                    workspace.active_repository,
                    Some(workspace.placement),
                    workspace.theme_mode,
                )
            }
        };
        // Every repository this window would open passes the entry gates
        // here, before any bootstrap is dispatched or deferred.
        let mut entry_denials = Vec::new();
        if view_mode == GitCometViewMode::Normal {
            if let Some(path) = initial_path.as_ref()
                && let gitcomet_extension_api::GateDecision::Deny { reason } =
                    super::extension_host::entry_decision(
                        path,
                        gitcomet_extension_api::EntryOrigin::CommandLine,
                        cx,
                    )
            {
                entry_denials.push(reason);
                initial_path = None;
            }
            let (allowed, denied) = super::extension_host::filter_entries(
                std::mem::take(&mut ui_session.open_repos),
                gitcomet_extension_api::EntryOrigin::WorkspaceRestore,
                cx,
            );
            ui_session.open_repos = allowed;
            entry_denials.extend(denied);
            if ui_session
                .active_repo
                .as_ref()
                .is_some_and(|active| !ui_session.open_repos.contains(active))
            {
                ui_session.active_repo = None;
            }
        }
        if should_seed_initial_repository_from_session(
            view_mode,
            initial_path.as_deref(),
            initial_repository_launch_mode,
            !ui_session.open_repos.is_empty(),
        ) && let Some(path) = initial_path.as_ref()
        {
            if !ui_session.open_repos.iter().any(|p| p == path) {
                ui_session.open_repos.push(path.clone());
            }
            ui_session.active_repo = Some(path.clone());
        }

        let ui_preferences = UiPreferences::from_session(&ui_session);
        let history_show_tags = ui_preferences.history.show_tags;
        let history_tag_fetch_mode = ui_preferences.history.tag_fetch_mode;
        let default_tag_type = ui_preferences.repository.default_tag_type;
        let remote_settings = RemoteSettings {
            prune_deleted_remote_branches_on_fetch: ui_preferences
                .remotes
                .prune_deleted_remote_branches_on_fetch,
        };
        store.dispatch(Msg::SetRemoteUrlPolicy(
            ui_preferences.security.remote_url_policy,
        ));
        store.dispatch(Msg::SetGitLogSettings {
            show_history_tags: history_show_tags,
            tag_fetch_mode: history_tag_fetch_mode,
            verify_commit_signatures: ui_preferences.history.verify_commit_signatures,
        });
        store.dispatch(Msg::SetDefaultTagType(default_tag_type));
        store.dispatch(Msg::SetRemoteSettings(remote_settings));
        store.dispatch(Msg::SetMaintenanceSettings(MaintenanceSettings {
            recommend: ui_preferences.maintenance.recommend,
        }));
        store.dispatch(Msg::SetFileBrowserSettings(FileBrowserSettings {
            follow_selected_commit: ui_preferences.history.files_follow_selected_commit,
        }));
        store.dispatch(Msg::SetLargeFileSettings(ui_preferences.large_files));
        let saved_open_repos = ui_session.open_repos.clone();
        let saved_active_repo = ui_session.active_repo.clone();
        let mut startup_repo_bootstrap_pending = false;
        let mut deferred_repo_bootstrap = None;

        // Directly constructed views may receive a store that is already populated.
        // Preparation itself happens once, even when its queued restore has not run
        // by the time the native window attaches the view.
        let initial_store_state = store.snapshot();
        let store_preloaded = !initial_store_state.repos.is_empty();
        let git_runtime_available = initial_store_state.git_runtime.is_available();
        let mut pending_repo_open_reservations = FxHashMap::default();
        let mut pending_repo_open_active = None;
        if initial_repository_launch_mode == InitialRepositoryLaunchMode::OpenExplicitly
            && let Some(path) = initial_path.as_ref()
        {
            let failure_revision = initial_store_state.repo_open_failure_revision;
            pending_repo_open_reservations.insert(
                path.clone(),
                PendingRepoOpen {
                    failure_revision,
                    persist_in_workspace: true,
                },
            );
            pending_repo_open_active = Some(path.clone());
        }
        let should_auto_restore = !crate::startup_probe::disable_auto_restore()
            && view_mode != GitCometViewMode::FocusedMergetool
            && crate::ui_runtime::current().auto_restores_session()
            && !store_preloaded;

        if should_auto_restore {
            if !saved_open_repos.is_empty() {
                if git_runtime_available {
                    store.dispatch(Msg::RestoreSession {
                        open_repos: saved_open_repos,
                        active_repo: saved_active_repo,
                    });
                    startup_repo_bootstrap_pending = true;
                } else {
                    deferred_repo_bootstrap = Some(DeferredRepoBootstrap::RestoreSession {
                        open_repos: saved_open_repos,
                        active_repo: saved_active_repo,
                    });
                }
            }
        } else if store_preloaded {
            if let Some(path) = initial_path.as_ref() {
                if git_runtime_available {
                    store.dispatch(Msg::OpenRepo(path.clone()));
                } else {
                    deferred_repo_bootstrap =
                        Some(DeferredRepoBootstrap::OpenRepos(vec![path.clone()]));
                }
            }
        } else if let Some(path) = initial_path.as_ref() {
            if git_runtime_available {
                store.dispatch(Msg::OpenRepo(path.clone()));
                startup_repo_bootstrap_pending = true;
            } else {
                deferred_repo_bootstrap =
                    Some(DeferredRepoBootstrap::OpenRepos(vec![path.clone()]));
            }
        }

        if deferred_repo_bootstrap.is_some() {
            startup_repo_bootstrap_pending = true;
        }

        gitcomet_core::startup_trace::mark(if deferred_repo_bootstrap.is_some() {
            "repository_bootstrap_deferred_for_git"
        } else if startup_repo_bootstrap_pending {
            "repository_bootstrap_dispatched"
        } else {
            "repository_bootstrap_not_needed"
        });
        config.initial_path = initial_path;
        Self {
            config,
            ui_session,
            ui_preferences,
            workspace_id,
            persisted_workspace_repo_paths,
            persisted_workspace_active_repository,
            initial_window_placement,
            workspace_theme_key,
            entry_denials,
            startup_repo_bootstrap_pending,
            startup_repo_open_failure_revision: initial_store_state.repo_open_failure_revision,
            deferred_repo_bootstrap,
            pending_repo_open_reservations,
            pending_repo_open_active,
            git_probe_handoff: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::path_utils::canonicalize_or_original;
    use gitcomet_core::process::{
        GitExecutableAvailability, GitExecutablePreference, GitRuntimeState,
    };
    use gitcomet_core::services::{GitBackend, GitRepository};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct RecordingBackend {
        opens: Arc<AtomicUsize>,
        fail: bool,
    }

    impl GitBackend for RecordingBackend {
        fn open(&self, workdir: &Path) -> gitcomet_core::services::Result<Arc<dyn GitRepository>> {
            self.opens.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err(gitcomet_core::error::Error::new(
                    gitcomet_core::error::ErrorKind::NotARepository,
                ));
            }
            Ok(Arc::new(
                gitcomet_core::test_support::UnconfiguredRepository::new(workdir.to_path_buf()),
            ))
        }
    }

    fn store(
        available: bool,
        fail: bool,
    ) -> (
        AppStore,
        smol::channel::Receiver<StoreEvent>,
        Arc<AtomicUsize>,
    ) {
        let opens = Arc::new(AtomicUsize::new(0));
        let (store, events) = AppStore::new_test(Arc::new(RecordingBackend {
            opens: Arc::clone(&opens),
            fail,
        }));
        store.disable_repo_monitors_for_test();
        store.replace_snapshot_for_test(Arc::new(AppState {
            git_runtime: GitRuntimeState {
                preference: GitExecutablePreference::SystemPath,
                availability: if available {
                    GitExecutableAvailability::Available {
                        version_output: "git version 2.55.0".into(),
                    }
                } else {
                    GitExecutableAvailability::Checking
                },
            },
            ..AppState::test_default()
        }));
        (store, events, opens)
    }

    fn restored_config(paths: Vec<PathBuf>) -> GitCometViewConfig {
        let mut workspace = session::Workspace::new(paths);
        workspace.active_repository = workspace.repositories.first().cloned();
        GitCometViewConfig {
            workspace: WorkspaceBootstrap::Saved(Box::new(workspace)),
            ..GitCometViewConfig::normal(None)
        }
    }

    fn prepare(
        store: &AppStore,
        config: GitCometViewConfig,
        session: session::UiSession,
        cx: &mut gpui::TestAppContext,
    ) -> PreparedWindowBootstrap {
        crate::ui_runtime::with_override(
            crate::ui_runtime::UiRuntime::deterministic_auto_restore(),
            || cx.update(|app| PreparedWindowBootstrap::from_session(store, config, session, app)),
        )
    }

    fn wait_for_state(store: &AppStore, ready: impl Fn(&AppState) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !ready(&store.snapshot()) {
            assert!(Instant::now() < deadline, "bootstrap did not settle");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn shutdown(store: &AppStore) {
        store
            .shutdown_for_test()
            .recv_timeout(Duration::from_secs(3))
            .expect("store and its effects shut down");
    }

    fn flush_feedback_acknowledgments(store: &AppStore) {
        // The worker coalesces acknowledgments until the next reducer pass.
        // A test barrier alone deliberately does not publish a new state.
        store.dispatch(Msg::SetDefaultTagType(store.snapshot().default_tag_type));
        store
            .barrier_for_test()
            .recv_timeout(Duration::from_secs(3))
            .expect("feedback acknowledgments are reduced");
    }

    #[gpui::test]
    fn empty_home_adopts_git_before_attachment_and_renders_usable_actions(
        cx: &mut gpui::TestAppContext,
    ) {
        let (store, events, opens) = store(false, false);
        let mut bootstrap = prepare(
            &store,
            GitCometViewConfig {
                workspace: WorkspaceBootstrap::Empty,
                ..GitCometViewConfig::normal(None)
            },
            session::UiSession {
                open_repos: vec![PathBuf::from("/repos/legacy-home-session")],
                ..Default::default()
            },
            cx,
        );
        assert!(!bootstrap.startup_repo_bootstrap_pending);
        assert!(bootstrap.deferred_repo_bootstrap.is_none());
        let handoff = bootstrap
            .begin_git_probe_handoff(&store)
            .expect("empty Home must adopt the running Git probe")
            .upgrade()
            .unwrap();
        assert!(
            bootstrap.begin_git_probe_handoff(&store).is_none(),
            "preparation must not replace an already-owned handoff"
        );
        let runtime = AppState::test_default().git_runtime;
        // The main thread remains occupied creating the native window. Home's
        // store must become usable without a foreground runtime-report task.
        std::thread::scope(|scope| {
            scope.spawn(|| handoff.complete(runtime.clone(), &runtime));
        });
        wait_for_state(&store, |state| state.git_runtime == runtime);
        handoff.complete(runtime.clone(), &runtime);
        let store_for_assert = store.clone();
        let (view, cx) = cx.add_window_view(|window, cx| {
            GitCometView::new_with_bootstrap(store, events, bootstrap, window, cx)
        });
        cx.update(|window, app| {
            let view = view.read(app);
            assert!(view.state.git_runtime.is_available());
            assert!(view.is_home_screen_active());
            assert!(!view.blocks_repository_management_actions());
            assert!(!view.startup_repo_bootstrap_pending);
            assert!(view.deferred_repo_bootstrap.is_none());
            assert!(view.startup_repo_runtime_adoption.is_none());
            let _ = window.draw(app);
        });
        for selector in [
            "home_open_repo_action",
            "home_clone_repo_action",
            "home_init_repo_action",
        ] {
            assert!(
                cx.debug_bounds(selector).is_some(),
                "the first attached Home must render usable {selector}"
            );
        }
        assert!(cx.debug_bounds("repository_loading_screen").is_none());
        assert!(cx.debug_bounds("git_unavailable_screen").is_none());
        handoff.complete(runtime.clone(), &runtime);
        shutdown(&store_for_assert);
        assert!(store_for_assert.snapshot().repos.is_empty());
        assert_eq!(opens.load(Ordering::SeqCst), 0);
    }

    #[gpui::test]
    fn empty_home_rejects_missing_and_stale_git_then_recovers_after_attachment(
        cx: &mut gpui::TestAppContext,
    ) {
        let (store, events, opens) = store(false, false);
        let mut bootstrap = prepare(
            &store,
            GitCometViewConfig::normal(None),
            session::UiSession::default(),
            cx,
        );
        let handoff = bootstrap
            .begin_git_probe_handoff(&store)
            .unwrap()
            .upgrade()
            .unwrap();
        let runtime = AppState::test_default().git_runtime;
        let mut missing = runtime.clone();
        missing.availability = GitExecutableAvailability::Unavailable {
            detail: "Git is unavailable".into(),
        };
        handoff.complete(missing.clone(), &missing);
        handoff.complete(runtime.clone(), &missing);
        let mut other = runtime.clone();
        other.preference = GitExecutablePreference::Custom(PathBuf::from("/other/home-git"));
        handoff.complete(other.clone(), &other);
        store
            .barrier_for_test()
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        assert!(matches!(
            store.snapshot().git_runtime.availability,
            GitExecutableAvailability::Checking
        ));

        let store_for_assert = store.clone();
        let (view, cx) = cx.add_window_view(|window, cx| {
            GitCometView::new_with_bootstrap(store, events, bootstrap, window, cx)
        });
        // Attachment owns reporting now; even a valid late worker result must
        // not make unavailable Git appear usable or change its recovery path.
        handoff.complete(runtime.clone(), &runtime);
        store_for_assert
            .barrier_for_test()
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        assert!(!store_for_assert.snapshot().git_runtime.is_available());
        store_for_assert.dispatch(Msg::SetGitRuntimeState(missing));
        wait_for_state(&store_for_assert, |state| {
            state.git_runtime.unavailable_detail().is_some()
        });
        cx.update(|window, app| {
            view.update(app, |view, cx| {
                view.apply_state_snapshot(store_for_assert.snapshot(), cx);
                assert!(view.blocks_repository_management_actions());
                assert!(view.deferred_repo_bootstrap.is_none());
            });
            let _ = window.draw(app);
        });
        assert!(cx.debug_bounds("git_unavailable_screen").is_some());
        assert!(cx.debug_bounds("home_open_repo_action").is_none());

        store_for_assert.dispatch(Msg::SetGitRuntimeState(runtime.clone()));
        wait_for_state(&store_for_assert, |state| state.git_runtime == runtime);
        cx.update(|window, app| {
            view.update(app, |view, cx| {
                view.apply_state_snapshot(store_for_assert.snapshot(), cx);
                assert!(view.state.git_runtime.is_available());
            });
            let _ = window.draw(app);
            // Rendering reconciles the Git-unavailable window gate with the
            // recovered snapshot, just as it does for ordinary runtime recovery.
            assert!(!view.read(app).blocks_repository_management_actions());
        });
        assert!(cx.debug_bounds("home_open_repo_action").is_some());
        assert!(cx.debug_bounds("git_unavailable_screen").is_none());
        shutdown(&store_for_assert);
        assert_eq!(opens.load(Ordering::SeqCst), 0);
    }

    #[gpui::test]
    fn available_empty_home_does_not_wait_for_another_probe(cx: &mut gpui::TestAppContext) {
        let (store, _events, opens) = store(true, false);
        let mut bootstrap = prepare(
            &store,
            GitCometViewConfig::normal(None),
            session::UiSession::default(),
            cx,
        );
        assert!(bootstrap.begin_git_probe_handoff(&store).is_none());
        assert!(!bootstrap.startup_repo_bootstrap_pending);
        shutdown(&store);
        assert_eq!(opens.load(Ordering::SeqCst), 0);
    }

    #[gpui::test]
    fn git_probe_handoff_loads_before_attachment_and_dispatches_only_once(
        cx: &mut gpui::TestAppContext,
    ) {
        let (store, events, opens) = store(false, false);
        let directory = tempfile::tempdir().unwrap();
        let path = canonicalize_or_original(directory.path().to_path_buf());
        let mut bootstrap = prepare(
            &store,
            restored_config(vec![path.clone()]),
            session::UiSession::default(),
            cx,
        );
        let handoff = bootstrap
            .begin_git_probe_handoff(&store)
            .unwrap()
            .upgrade()
            .unwrap();
        let runtime = AppState::test_default().git_runtime;
        // Run completion on a worker without advancing the UI executor. This is
        // the interval in which the real main thread is initializing DX12.
        std::thread::scope(|scope| {
            scope.spawn(|| handoff.complete(runtime.clone(), &runtime));
        });
        wait_for_state(&store, |state| {
            state.repos.first().is_some_and(|repo| {
                repo.spec.workdir == path && matches!(repo.open, Loadable::Ready(()))
            })
        });
        let repo_id = store.snapshot().active_repo;
        handoff.complete(runtime.clone(), &runtime);
        let store_for_assert = store.clone();
        let (view, cx) = cx.add_window_view(|window, cx| {
            GitCometView::new_with_bootstrap(store, events, bootstrap, window, cx)
        });
        cx.update(|_, app| {
            let view = view.read(app);
            assert_eq!(view.state.active_repo, repo_id);
            assert!(view.deferred_repo_bootstrap.is_none());
            assert!(view.startup_repo_runtime_adoption.is_none());
        });
        handoff.complete(runtime.clone(), &runtime);
        shutdown(&store_for_assert);
        assert_eq!(opens.load(Ordering::SeqCst), 1);
    }

    #[gpui::test]
    fn git_probe_handoff_attachment_wins_and_late_completion_leaves_recovery_to_view(
        cx: &mut gpui::TestAppContext,
    ) {
        let (store, events, opens) = store(false, true);
        let path = PathBuf::from("/repos/late-bootstrap-probe");
        let mut bootstrap = prepare(
            &store,
            restored_config(vec![path.clone()]),
            session::UiSession::default(),
            cx,
        );
        let handoff = bootstrap
            .begin_git_probe_handoff(&store)
            .unwrap()
            .upgrade()
            .unwrap();
        let store_for_assert = store.clone();
        let (view, cx) = cx.add_window_view(|window, cx| {
            GitCometView::new_with_bootstrap(store, events, bootstrap, window, cx)
        });
        let runtime = AppState::test_default().git_runtime;
        handoff.complete(runtime.clone(), &runtime);
        store_for_assert
            .barrier_for_test()
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        assert!(!store_for_assert.snapshot().git_runtime.is_available());
        assert_eq!(opens.load(Ordering::SeqCst), 0);
        store_for_assert.dispatch(Msg::SetGitRuntimeState(runtime));
        store_for_assert
            .barrier_for_test()
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                view.apply_state_snapshot(store_for_assert.snapshot(), cx);
                assert!(view.deferred_repo_bootstrap.is_none());
            });
        });
        wait_for_state(&store_for_assert, |state| {
            state.repo_open_failure_revision != 0
        });
        shutdown(&store_for_assert);
        assert_eq!(opens.load(Ordering::SeqCst), 1);
    }

    #[gpui::test]
    fn git_probe_handoff_rejects_missing_or_stale_results_and_keeps_restore(
        cx: &mut gpui::TestAppContext,
    ) {
        let (store, _events, opens) = store(false, false);
        let path = PathBuf::from("/repos/bootstrap-stale-git");
        let mut bootstrap = prepare(
            &store,
            restored_config(vec![path]),
            session::UiSession::default(),
            cx,
        );
        let deferred = bootstrap.deferred_repo_bootstrap.clone();
        let handoff = bootstrap
            .begin_git_probe_handoff(&store)
            .unwrap()
            .upgrade()
            .unwrap();
        let runtime = AppState::test_default().git_runtime;
        let mut missing = runtime.clone();
        missing.availability = GitExecutableAvailability::Unavailable {
            detail: "Git is unavailable".into(),
        };
        handoff.complete(missing.clone(), &missing);
        handoff.complete(runtime.clone(), &missing);
        let mut another = runtime.clone();
        another.preference = GitExecutablePreference::Custom(PathBuf::from("/other/git"));
        handoff.complete(another.clone(), &another);
        // An already-reported store change must not be overwritten either.
        store.dispatch(Msg::SetGitRuntimeState(another.clone()));
        store
            .barrier_for_test()
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        handoff.complete(runtime.clone(), &runtime);
        assert!(bootstrap.attach_git_probe_handoff().is_none());
        assert_eq!(bootstrap.deferred_repo_bootstrap, deferred);
        assert_eq!(store.snapshot().git_runtime, another);
        shutdown(&store);
        assert_eq!(opens.load(Ordering::SeqCst), 0);
    }

    #[gpui::test]
    fn dropping_prepared_window_releases_git_probe_handoff(cx: &mut gpui::TestAppContext) {
        let opens = Arc::new(AtomicUsize::new(0));
        let backend = Arc::new(RecordingBackend {
            opens: opens.clone(),
            fail: false,
        });
        let backend_lifetime = Arc::downgrade(&backend);
        let (store, _events) = AppStore::new_test(backend);
        store.disable_repo_monitors_for_test();
        let mut state = (*store.snapshot()).clone();
        state.git_runtime.availability = GitExecutableAvailability::Checking;
        store.replace_snapshot_for_test(Arc::new(state));
        let mut bootstrap = prepare(
            &store,
            restored_config(vec![PathBuf::from("/repos/abandoned-bootstrap")]),
            session::UiSession::default(),
            cx,
        );
        let worker = bootstrap.begin_git_probe_handoff(&store).unwrap();
        store
            .barrier_for_test()
            .recv_timeout(Duration::from_secs(3))
            .expect("store initialization and bootstrap settings are reduced");
        drop(store);
        assert!(worker.upgrade().is_some());
        drop(bootstrap);
        assert!(worker.upgrade().is_none());
        let deadline = Instant::now() + Duration::from_secs(3);
        while backend_lifetime.upgrade().is_some() {
            assert!(
                Instant::now() < deadline,
                "an abandoned handoff must release the store and its workers"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(opens.load(Ordering::SeqCst), 0);
    }

    #[gpui::test]
    fn independent_windows_adopt_one_probe_for_session_and_focused_open(
        cx: &mut gpui::TestAppContext,
    ) {
        let (session_store, _session_events, session_opens) = store(false, false);
        let (focused_store, _focused_events, focused_opens) = store(false, false);
        let session_directory = tempfile::tempdir().unwrap();
        let focused_directory = tempfile::tempdir().unwrap();
        let session_path = canonicalize_or_original(session_directory.path().to_path_buf());
        let focused_path = canonicalize_or_original(focused_directory.path().to_path_buf());
        let mut session_bootstrap = prepare(
            &session_store,
            restored_config(vec![session_path.clone()]),
            session::UiSession::default(),
            cx,
        );
        let mut focused_bootstrap = prepare(
            &focused_store,
            GitCometViewConfig {
                initial_path: Some(focused_path.clone()),
                view_mode: GitCometViewMode::FocusedMergetool,
                ..Default::default()
            },
            session::UiSession {
                open_repos: vec![session_path],
                ..Default::default()
            },
            cx,
        );
        let session_handoff = session_bootstrap
            .begin_git_probe_handoff(&session_store)
            .unwrap()
            .upgrade()
            .unwrap();
        let focused_handoff = focused_bootstrap
            .begin_git_probe_handoff(&focused_store)
            .unwrap()
            .upgrade()
            .unwrap();
        let runtime = AppState::test_default().git_runtime;
        std::thread::scope(|scope| {
            scope.spawn(|| session_handoff.complete(runtime.clone(), &runtime));
            scope.spawn(|| focused_handoff.complete(runtime.clone(), &runtime));
        });
        for store in [&session_store, &focused_store] {
            wait_for_state(store, |state| {
                state
                    .repos
                    .first()
                    .is_some_and(|repo| matches!(repo.open, Loadable::Ready(())))
            });
        }
        assert_eq!(
            session_bootstrap.attach_git_probe_handoff(),
            Some(runtime.clone())
        );
        assert_eq!(focused_bootstrap.attach_git_probe_handoff(), Some(runtime));
        assert_eq!(session_store.snapshot().repos.len(), 1);
        assert_eq!(focused_store.snapshot().repos.len(), 1);
        assert_eq!(
            focused_store.snapshot().repos[0].spec.workdir,
            focused_path,
            "a focused tool must not restore the browser's session tabs"
        );
        shutdown(&session_store);
        shutdown(&focused_store);
        assert_eq!(session_opens.load(Ordering::SeqCst), 1);
        assert_eq!(focused_opens.load(Ordering::SeqCst), 1);
    }

    #[gpui::test]
    fn coalesced_bootstrap_runtime_does_not_reload_but_later_git_recovery_does(
        cx: &mut gpui::TestAppContext,
    ) {
        let (store, events, _) = store(false, false);
        let directory = tempfile::tempdir().unwrap();
        let mut bootstrap = prepare(
            &store,
            restored_config(vec![directory.path().to_path_buf()]),
            session::UiSession::default(),
            cx,
        );
        let initial = store.snapshot();
        let handoff = bootstrap
            .begin_git_probe_handoff(&store)
            .unwrap()
            .upgrade()
            .unwrap();
        let runtime = AppState::test_default().git_runtime;
        handoff.complete(runtime.clone(), &runtime);
        wait_for_state(&store, |state| {
            state.repos.first().is_some_and(|repo| {
                matches!(repo.open, Loadable::Ready(())) && !repo.loads_in_flight.any_in_flight()
            })
        });
        let mut loaded = (*store.snapshot()).clone();
        let marker = gitcomet_state::model::ComparisonMark {
            commit_id: CommitId("bootstrap-retained-mark".into()),
            label: "retained mark".into(),
        };
        loaded.repos[0].navigation.comparison_mark = Some(marker.clone());
        // Reproduce attachment before the queued adoption is published, followed
        // by a coalesced snapshot containing the already-loaded repository.
        store.replace_snapshot_for_test(initial);
        let store_for_assert = store.clone();
        let (view, cx) = cx.add_window_view(|window, cx| {
            GitCometView::new_with_bootstrap(store, events, bootstrap, window, cx)
        });
        store_for_assert.replace_snapshot_for_test(Arc::new(loaded.clone()));
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                assert_eq!(view.startup_repo_runtime_adoption.as_ref(), Some(&runtime));
                view.apply_state_snapshot(store_for_assert.snapshot(), cx);
                assert!(view.startup_repo_runtime_adoption.is_none());
            });
        });
        store_for_assert
            .barrier_for_test()
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        assert_eq!(
            store_for_assert.snapshot().repos[0]
                .navigation
                .comparison_mark,
            Some(marker),
            "a duplicate ReloadRepo would clear the retained comparison"
        );
        let mut missing = loaded.clone();
        missing.git_runtime.availability = GitExecutableAvailability::Unavailable {
            detail: "Git temporarily missing".into(),
        };
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                view.apply_state_snapshot(Arc::new(missing), cx);
                view.apply_state_snapshot(Arc::new(loaded), cx);
            });
        });
        store_for_assert
            .barrier_for_test()
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        assert!(
            store_for_assert.snapshot().repos[0]
                .navigation
                .comparison_mark
                .is_none(),
            "later runtime recovery must still reload the repository"
        );
        shutdown(&store_for_assert);
    }

    #[gpui::test]
    fn restored_repository_loads_before_window_and_attaches_without_reopening(
        cx: &mut gpui::TestAppContext,
    ) {
        let (store, events, opens) = store(true, false);
        let active_directory = tempfile::tempdir().unwrap();
        let inactive_directory = tempfile::tempdir().unwrap();
        let path = canonicalize_or_original(active_directory.path().to_path_buf());
        let inactive = canonicalize_or_original(inactive_directory.path().to_path_buf());
        let bootstrap = prepare(
            &store,
            restored_config(vec![path.clone(), inactive.clone()]),
            session::UiSession {
                history_show_tags: Some(false),
                history_verify_commit_signatures: Some(false),
                file_browser_follow_selected_commit: Some(true),
                ..Default::default()
            },
            cx,
        );
        wait_for_state(&store, |state| {
            state
                .repos
                .iter()
                .any(|repo| repo.spec.workdir == path && matches!(repo.open, Loadable::Ready(())))
        });
        let state = store.snapshot();
        let active_id = state.active_repo.expect("restored active repository");
        assert_eq!(state.repos.len(), 2);
        assert!(!state.git_log_settings.show_history_tags);
        assert!(!state.git_log_settings.verify_commit_signatures);
        assert!(state.file_browser_settings.follow_selected_commit);
        assert!(matches!(
            state
                .repos
                .iter()
                .find(|repo| repo.spec.workdir == inactive)
                .unwrap()
                .open,
            Loadable::NotLoaded
        ));
        assert!(
            !events.is_empty(),
            "the live poller must observe the preloaded state"
        );
        let store_for_assert = store.clone();
        let (view, cx) = cx.add_window_view(|window, cx| {
            GitCometView::new_with_bootstrap(store, events, bootstrap, window, cx)
        });
        cx.update(|_, app| {
            let view = view.read(app);
            assert_eq!(view.state.active_repo, Some(active_id));
            assert!(!view.startup_repo_bootstrap_pending);
        });
        shutdown(&store_for_assert);
        assert_eq!(
            opens.load(Ordering::SeqCst),
            1,
            "only the active tab opens once"
        );
        assert_eq!(store_for_assert.snapshot().active_repo, Some(active_id));
    }

    #[gpui::test]
    fn failed_prepared_open_is_not_retried_when_empty_store_attaches(
        cx: &mut gpui::TestAppContext,
    ) {
        let (store, events, opens) = store(false, true);
        let mut bootstrap = prepare(
            &store,
            restored_config(vec![PathBuf::from("/repos/bootstrap-failure")]),
            session::UiSession::default(),
            cx,
        );
        let handoff = bootstrap
            .begin_git_probe_handoff(&store)
            .unwrap()
            .upgrade()
            .unwrap();
        let runtime = AppState::test_default().git_runtime;
        handoff.complete(runtime.clone(), &runtime);
        drop(handoff);
        wait_for_state(&store, |state| state.repo_open_failure_revision != 0);
        assert!(store.snapshot().repos.is_empty());
        let store_for_assert = store.clone();
        let (view, cx) = cx.add_window_view(|window, cx| {
            GitCometView::new_with_bootstrap(store, events, bootstrap, window, cx)
        });
        cx.update(|_, app| {
            let view = view.read(app);
            assert!(!view.startup_repo_bootstrap_pending);
            let toasts = view.toast_host.read(app).toasts_for_tests(app);
            assert_eq!(
                toasts
                    .iter()
                    .filter(|(kind, message)| {
                        *kind == components::ToastKind::Warning
                            && message.contains("No valid Git repository")
                    })
                    .count(),
                1,
                "the pre-attachment failure must be reported once"
            );
        });
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                view.apply_state_snapshot(store_for_assert.snapshot(), cx);
            });
        });
        cx.update(|_, app| {
            let toasts = view.read(app).toast_host.read(app).toasts_for_tests(app);
            assert_eq!(
                toasts
                    .iter()
                    .filter(|(kind, message)| {
                        *kind == components::ToastKind::Warning
                            && message.contains("No valid Git repository")
                    })
                    .count(),
                1,
                "the first poll must not repeat feedback already shown on attachment"
            );
        });
        flush_feedback_acknowledgments(&store_for_assert);
        assert!(store_for_assert.snapshot().notifications.is_empty());
        shutdown(&store_for_assert);
        assert_eq!(opens.load(Ordering::SeqCst), 1);
    }

    #[gpui::test]
    fn entry_gates_filter_bootstrap_before_any_window_exists(cx: &mut gpui::TestAppContext) {
        use gitcomet_extension_api::{Extension, ExtensionId, GateDecision, Registrar};
        const DENY_MARKER: &str = ".bootstrap-entry-denied";
        struct DenyMarkedRepository;
        impl Extension for DenyMarkedRepository {
            fn id(&self) -> ExtensionId {
                ExtensionId::new("test.bootstrap-entry").unwrap()
            }

            fn register(&self, registrar: &mut Registrar) {
                registrar.repository_entry_gate(
                    "deny-marker",
                    std::rc::Rc::new(|request, app| {
                        assert!(
                            app.windows().is_empty(),
                            "gate runs before the first window"
                        );
                        if request.path.join(DENY_MARKER).is_file() {
                            GateDecision::Deny {
                                reason: "bootstrap entry denied".into(),
                            }
                        } else {
                            GateDecision::Allow
                        }
                    }),
                );
            }
        }
        let registry =
            gitcomet_extension_api::Registry::build(vec![Box::new(DenyMarkedRepository)]).unwrap();
        cx.update(|app| super::super::extension_host::install_registry(registry, app));
        let denied = tempfile::tempdir().unwrap();
        std::fs::write(denied.path().join(DENY_MARKER), "").unwrap();
        let denied_path = canonicalize_or_original(denied.path().to_path_buf());
        let allowed_directory = tempfile::tempdir().unwrap();
        let allowed = canonicalize_or_original(allowed_directory.path().to_path_buf());
        let (store, _events, opens) = store(false, false);
        let mut config = restored_config(vec![denied_path.clone(), allowed.clone()]);
        config.initial_path = Some(denied_path);
        let mut bootstrap = prepare(&store, config, session::UiSession::default(), cx);
        assert_eq!(bootstrap.entry_denials.len(), 2);
        assert!(bootstrap.config.initial_path.is_none());
        assert_eq!(bootstrap.ui_session.open_repos, vec![allowed.clone()]);
        assert!(bootstrap.ui_session.active_repo.is_none());
        let handoff = bootstrap
            .begin_git_probe_handoff(&store)
            .unwrap()
            .upgrade()
            .unwrap();
        let runtime = AppState::test_default().git_runtime;
        handoff.complete(runtime.clone(), &runtime);
        wait_for_state(&store, |state| {
            state.repos.iter().any(|repo| {
                repo.spec.workdir == allowed && matches!(repo.open, Loadable::Ready(()))
            })
        });
        shutdown(&store);
        assert_eq!(opens.load(Ordering::SeqCst), 1);
        assert_eq!(store.snapshot().repos.len(), 1);
    }

    #[gpui::test]
    fn unsettled_git_recovers_and_a_coalesced_failure_clears_loading(
        cx: &mut gpui::TestAppContext,
    ) {
        let (store, events, opens) = store(false, true);
        let path = PathBuf::from("/repos/bootstrap-awaits-git");
        let bootstrap = prepare(
            &store,
            restored_config(vec![path.clone()]),
            session::UiSession::default(),
            cx,
        );
        assert!(bootstrap.startup_repo_bootstrap_pending);
        assert_eq!(
            bootstrap.deferred_repo_bootstrap,
            Some(DeferredRepoBootstrap::RestoreSession {
                open_repos: vec![path.clone()],
                active_repo: Some(path.clone()),
            })
        );
        assert_eq!(opens.load(Ordering::SeqCst), 0);
        assert!(store.snapshot().repos.is_empty());
        let store_for_assert = store.clone();
        let (view, cx) = cx.add_window_view(|window, cx| {
            GitCometView::new_with_bootstrap(store, events, bootstrap, window, cx)
        });
        store_for_assert.dispatch(Msg::SetGitRuntimeState(
            AppState::test_default().git_runtime,
        ));
        store_for_assert
            .barrier_for_test()
            .recv_timeout(Duration::from_secs(3))
            .expect("runtime recovery is reduced");
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                view.apply_state_snapshot(store_for_assert.snapshot(), cx);
                assert!(view.deferred_repo_bootstrap.is_none());
            });
        });
        // Do not apply the intermediate opening tab: live notifications can
        // coalesce the entire open/failure while a frame is being rendered.
        wait_for_state(&store_for_assert, |state| {
            state.repo_open_failure_revision != 0
        });
        assert!(store_for_assert.snapshot().repos.is_empty());
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                view.apply_state_snapshot(store_for_assert.snapshot(), cx);
                assert!(!view.startup_repo_bootstrap_pending);
            });
        });
        shutdown(&store_for_assert);
        assert_eq!(opens.load(Ordering::SeqCst), 1);
    }

    #[gpui::test]
    fn retained_diagnostics_are_reported_and_acknowledged_once_on_attachment(
        cx: &mut gpui::TestAppContext,
    ) {
        let (store, events, opens) = store(true, false);
        let mut state = (*store.snapshot()).clone();
        let repo_id = RepoId(73);
        let mut repo = RepoState::new_opening(
            repo_id,
            gitcomet_core::domain::RepoSpec {
                workdir: PathBuf::from("/repos/preloaded-diagnostic"),
            },
        );
        repo.open = Loadable::Ready(());
        repo.feedback_mut()
            .push_diagnostic(gitcomet_state::model::DiagnosticEntry {
                time: std::time::SystemTime::now(),
                kind: DiagnosticKind::Error,
                message: "preloaded diagnostic".into(),
            });
        state.repos.push(repo);
        state.active_repo = Some(repo_id);
        store.replace_snapshot_for_test(Arc::new(state));
        let bootstrap = prepare(
            &store,
            GitCometViewConfig::normal(None),
            session::UiSession::default(),
            cx,
        );
        let store_for_assert = store.clone();
        let (view, cx) = cx.add_window_view(|window, cx| {
            GitCometView::new_with_bootstrap(store, events, bootstrap, window, cx)
        });
        cx.update(|_, app| {
            let toasts = view.read(app).toast_host.read(app).toasts_for_tests(app);
            assert_eq!(
                toasts
                    .iter()
                    .filter(|(_, text)| text == "preloaded diagnostic")
                    .count(),
                1
            );
            view.update(app, |view, cx| {
                view.apply_state_snapshot(store_for_assert.snapshot(), cx);
            });
            let toasts = view.read(app).toast_host.read(app).toasts_for_tests(app);
            assert_eq!(
                toasts
                    .iter()
                    .filter(|(_, text)| text == "preloaded diagnostic")
                    .count(),
                1
            );
        });
        flush_feedback_acknowledgments(&store_for_assert);
        let remaining_diagnostics = store_for_assert.snapshot().repos[0]
            .feedback
            .diagnostics
            .clone();
        // This synthetic preloaded repository has no actor handle, so later
        // view requests can add unrelated diagnostics. Acknowledging the entry
        // shown above must release that entry without assuming the queue is idle.
        assert!(
            !remaining_diagnostics
                .iter()
                .any(|entry| entry.message == "preloaded diagnostic"),
            "preloaded diagnostic was not acknowledged: {remaining_diagnostics:?}"
        );
        shutdown(&store_for_assert);
        assert_eq!(opens.load(Ordering::SeqCst), 0);
    }

    #[gpui::test]
    fn empty_workspaces_and_focused_tools_do_not_restore_legacy_tabs(
        cx: &mut gpui::TestAppContext,
    ) {
        let legacy_path = PathBuf::from("/repos/legacy-session");
        let focused_path = PathBuf::from("/repos/focused-tool");
        let session = session::UiSession {
            open_repos: vec![legacy_path.clone()],
            active_repo: Some(legacy_path),
            ..Default::default()
        };
        let (store, _events, opens) = store(false, false);
        let mut empty = prepare(
            &store,
            GitCometViewConfig {
                workspace: WorkspaceBootstrap::Empty,
                ..GitCometViewConfig::normal(None)
            },
            session.clone(),
            cx,
        );
        assert!(empty.ui_session.open_repos.is_empty());
        assert!(empty.deferred_repo_bootstrap.is_none());
        assert!(!empty.startup_repo_bootstrap_pending);
        assert!(
            empty.begin_git_probe_handoff(&store).is_some(),
            "an empty workspace adopts Git without restoring legacy tabs"
        );
        assert!(empty.attach_git_probe_handoff().is_none());
        assert!(empty.deferred_repo_bootstrap.is_none());

        let mut focused = prepare(
            &store,
            GitCometViewConfig {
                initial_path: Some(focused_path.clone()),
                view_mode: GitCometViewMode::FocusedMergetool,
                ..Default::default()
            },
            session,
            cx,
        );
        assert_eq!(
            focused.deferred_repo_bootstrap,
            Some(DeferredRepoBootstrap::OpenRepos(vec![focused_path]))
        );
        let deferred = focused.deferred_repo_bootstrap.clone();
        assert!(focused.begin_git_probe_handoff(&store).is_some());
        assert!(focused.attach_git_probe_handoff().is_none());
        assert_eq!(focused.deferred_repo_bootstrap, deferred);
        shutdown(&store);
        assert_eq!(opens.load(Ordering::SeqCst), 0);
    }
}
