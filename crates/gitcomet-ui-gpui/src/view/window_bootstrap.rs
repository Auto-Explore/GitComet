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
            auto_fetch: ui_preferences.remotes.auto_fetch,
        };
        store.set_auto_fetch_allowed(config.view_mode == GitCometViewMode::Normal);
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
        store.dispatch(Msg::SetHistoryHiddenRefs(
            ui_preferences
                .history
                .hidden_ref_settings
                .filter(gitcomet_core::identity::current().history_ref_presets()),
        ));
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
mod tests;
