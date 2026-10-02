use super::*;
use crate::view::test_support::TestBackend;
use chrome::{cursor_style_for_resize_edge, resize_edge};
use gitcomet_core::domain::{
    Branch, CommitId, FileEntry, FileEntryKind, Remote, RemoteBranch, RepoSpec, StashEntry,
    Submodule, SubmoduleStatus, Upstream, Worktree,
};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::process::{GitExecutableAvailability, GitExecutablePreference, GitRuntimeState};
use gitcomet_core::services::{GitBackend, GitRepository, Result};
use gitcomet_core::test_support::git_fixture::FixtureTimer;
use gitcomet_state::model::{AppState, AuthPromptState, AuthRetryOperation, RepoId, RepoState};
use gitcomet_state::store::AppStore;
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

struct RecordingFailingBackend {
    opened: Arc<Mutex<Vec<PathBuf>>>,
}

impl GitBackend for RecordingFailingBackend {
    fn open(&self, workdir: &Path) -> Result<Arc<dyn GitRepository>> {
        self.opened
            .lock()
            .expect("recording backend lock")
            .push(workdir.to_path_buf());
        Err(Error::new(ErrorKind::Unsupported(
            "Recording backend does not open repositories",
        )))
    }
}

struct BlockingFailingBackend {
    release: Arc<(Mutex<bool>, Condvar)>,
}

impl GitBackend for BlockingFailingBackend {
    fn open(&self, _workdir: &Path) -> Result<Arc<dyn GitRepository>> {
        let (released, wake) = self.release.as_ref();
        let mut released = released.lock().expect("blocking backend gate lock");
        while !*released {
            released = wake.wait(released).expect("blocking backend gate wait");
        }
        Err(Error::new(ErrorKind::Unsupported(
            "Blocking backend does not open repositories",
        )))
    }
}

fn pump_for(cx: &mut gpui::VisualTestContext, duration: Duration) {
    let _timer = FixtureTimer::new("ui-wait", "timed-pump");
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(16));
    }
}

/// Like [`wait_until`], but keeps drawing and draining the test executor while
/// it waits.
///
/// Required whenever the awaited work is a GPUI task — a `cx.spawn(..).detach()`
/// — rather than something the store's own worker thread advances: those tasks
/// only run when the test driver pumps them, so a sleeping wait would spin out
/// its whole deadline without ever letting the task complete.
fn pump_until(
    cx: &mut gpui::VisualTestContext,
    description: &str,
    mut ready: impl FnMut(&mut gpui::VisualTestContext) -> bool,
) {
    let _timer = FixtureTimer::new("ui-wait", description);
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if ready(cx) {
            return;
        }
        if Instant::now() >= deadline {
            panic!("timed out waiting for {description}");
        }
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
        // Pumping can complete the awaited task synchronously. Do not impose
        // another real-time polling interval once the condition is satisfied.
        if ready(cx) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn wait_until(description: &str, ready: impl Fn() -> bool) {
    let _timer = FixtureTimer::new("ui-wait", description);
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if ready() {
            return;
        }
        if Instant::now() >= deadline {
            panic!("timed out waiting for {description}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn click_debug_selector(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    let center = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("expected {selector} to be rendered"))
        .center();
    cx.simulate_mouse_move(center, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(center, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_up(center, gpui::MouseButton::Left, gpui::Modifiers::default());
}

fn install_repo_tab_test_state(
    store: &AppStore,
    view: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
    active_repo: RepoId,
) {
    install_repo_tab_test_state_with_count(store, view, cx, active_repo, 3);
}

fn install_repo_tab_test_state_with_count(
    store: &AppStore,
    view: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
    active_repo: RepoId,
    repo_count: u64,
) {
    let mut state = AppState {
        active_repo: Some(active_repo),
        git_runtime: available_git_runtime_state(),
        ..AppState::test_default()
    };
    for ix in 1..=repo_count {
        state.repos.push(RepoState::new_opening(
            RepoId(ix),
            RepoSpec {
                workdir: PathBuf::from(format!("/tmp/repo-tab-menu-{ix}")),
            },
        ));
    }
    store.replace_snapshot_for_test(Arc::new(state));
    cx.update(|_window, app| {
        view.update(app, |this, cx| test_support::sync_store_snapshot(this, cx));
    });
    test_support::redraw(cx);
}

fn open_repo_tab_context_menu(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    let center = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("expected {selector} to be rendered"))
        .center();
    cx.simulate_mouse_move(center, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(center, gpui::MouseButton::Right, gpui::Modifiers::default());
    cx.simulate_mouse_up(center, gpui::MouseButton::Right, gpui::Modifiers::default());
    test_support::redraw(cx);
}

fn install_app_shortcuts_for_test(cx: &mut gpui::VisualTestContext, backend: Arc<dyn GitBackend>) {
    cx.update(|window, app| {
        crate::app::install_app_shortcuts_for_test(app, backend);
        let _ = window.draw(app);
        window.activate();
    });
}

fn sync_view_snapshot(cx: &mut gpui::VisualTestContext, view: &gpui::Entity<GitCometView>) {
    cx.update(|_window, app| {
        view.update(app, |this, cx| test_support::sync_store_snapshot(this, cx));
    });
    test_support::redraw(cx);
}

fn focus_detached_window_focus(cx: &mut gpui::VisualTestContext) {
    cx.update(|window, app| {
        let focus = app.focus_handle();
        window.focus(&focus, app);
        let _ = window.draw(app);
    });
    test_support::redraw(cx);
}

fn reveal_commit_is_open(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
) -> bool {
    cx.update(|_window, app| test_support::reveal_commit_is_open(view.read(app), app))
}

fn command_palette_is_open(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
) -> bool {
    cx.update(|_window, app| view.read(app).command_palette_open)
}

fn available_git_runtime_state() -> GitRuntimeState {
    GitRuntimeState {
        preference: GitExecutablePreference::SystemPath,
        availability: GitExecutableAvailability::Available {
            version_output: "git version 2.55.0".to_string(),
        },
    }
}

fn unavailable_git_runtime_state() -> GitRuntimeState {
    GitRuntimeState {
        preference: GitExecutablePreference::Custom(PathBuf::new()),
        availability: GitExecutableAvailability::Unavailable {
            detail: "Custom Git executable is not configured. Choose an executable or switch back to System PATH.".to_string(),
        },
    }
}

fn view_state_with_active_ready_repo(repo_id: RepoId) -> AppState {
    let mut repo = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo.open = Loadable::Ready(());
    AppState {
        repos: vec![repo],
        active_repo: Some(repo_id),
        ..AppState::test_default()
    }
}

fn repo_with_push_state(
    upstream: Option<Upstream>,
    remotes: Loadable<Arc<Vec<Remote>>>,
) -> RepoState {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/push-request"),
        },
    );
    repo.head_branch = Loadable::Ready("feature".to_string());
    repo.branches = Loadable::Ready(Arc::new(vec![Branch {
        name: "feature".to_string(),
        target: CommitId("deadbeef".into()),
        upstream,
        divergence: None,
    }]));
    repo.remotes = remotes;
    repo
}

fn open_repo_state_with_workdir(workdir: &str) -> RepoState {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: normalize_bootstrap_repo_path(PathBuf::from(workdir)),
        },
    );
    repo.open = Loadable::Ready(());
    repo
}

fn state_with_active_diff(path: &str, kind: FileStatusKind) -> AppState {
    let repo_id = RepoId(1);
    let path = PathBuf::from(path);
    let mut repo = open_repo_state_with_workdir("/repo");
    repo.worktree_status = Loadable::Ready(Arc::new(vec![FileStatus {
        path: path.clone(),
        kind,
        conflict: (kind == FileStatusKind::Conflicted)
            .then_some(gitcomet_core::domain::FileConflictKind::BothModified),
    }]));
    repo.diff_state.diff_target = Some(DiffTarget::working_tree(path, DiffArea::Unstaged));
    AppState {
        active_repo: Some(repo_id),
        repos: vec![repo],
        ..AppState::test_default()
    }
}

mod open_remote_in_browser;

/// A Home window with the given saved workspaces and recent repositories, and
/// the text-input keys bound so arrows reach the search box.
fn home_view_with(
    cx: &mut gpui::TestAppContext,
    workspaces: Vec<gitcomet_state::session::Workspace>,
    recents: Vec<PathBuf>,
) -> (gpui::Entity<GitCometView>, &mut gpui::VisualTestContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    cx.update(|app| crate::workspaces::initialize_for_test(app, workspaces));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        view.update(app, |view, _cx| {
            view.home_recent_repos = recents;
            view.home_pinned_repos.clear();
        });
        let _ = window.draw(app);
    });
    (view, cx)
}

fn press(cx: &mut gpui::VisualTestContext, keys: &str) {
    cx.simulate_keystrokes(keys);
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
}

fn dispatch_file_drop(cx: &mut gpui::VisualTestContext, event: gpui::FileDropEvent) {
    cx.update(|window, app| {
        let _ = window.dispatch_event(gpui::PlatformInput::FileDrop(event), app);
        let _ = window.draw(app);
    });
    cx.run_until_parked();
}

fn named_saved_workspace(name: &str, repo: &str) -> gitcomet_state::session::Workspace {
    let mut workspace = gitcomet_state::session::Workspace::new(vec![PathBuf::from(repo)]);
    workspace.custom_name = Some(name.to_string());
    workspace.restore_on_launch = false;
    workspace
}

mod extension_signals;
mod extension_views;
mod extensions;
mod focused_diff_host;
mod hosted_file_lists;
mod hosted_panes;
mod invalidation;
mod notifications;
mod previews;
mod routing_palette;
mod selection;
mod sidebar_layout;
mod window_lifecycle;
