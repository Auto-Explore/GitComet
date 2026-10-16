use crate::view::components;
use crate::view::test_support::TestBackend;
use crate::{theme::AppTheme, ui_scale, view};
use gitcomet_core::domain::*;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{GitBackend, GitRepository, PullMode, Result};
use gitcomet_state::model::Loadable;
use gitcomet_state::model::RepoId;
use gitcomet_state::model::SidebarDataRequest;
use gitcomet_state::msg::Msg;
use gitcomet_state::store::AppStore;
use gpui::prelude::*;
use gpui::{
    ClipboardItem, Decorations, KeyBinding, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent,
    Pixels, ScrollDelta, ScrollHandle, ScrollWheelEvent, Tiling, div, px,
};
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

struct SmokeView {
    theme: AppTheme,
    input: gpui::Entity<components::TextInput>,
}

impl SmokeView {
    fn new(window: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> Self {
        window.activate();
        let input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Enter".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        Self {
            theme: AppTheme::gitcomet_dark(),
            input,
        }
    }
}

impl gpui::Render for SmokeView {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        _cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        let theme = self.theme;
        let tabs = components::TabBar::new("smoke_tabs")
            .tab(
                components::Tab::new(("t", 0u64))
                    .selected(true)
                    .child(
                        div()
                            .debug_selector(|| "smoke_selected_tab_content".to_string())
                            .child("One"),
                    )
                    .render(theme)
                    .debug_selector(|| "smoke_selected_tab".to_string()),
            )
            .tab(
                components::Tab::new(("t", 1u64))
                    .selected(false)
                    .child(
                        div()
                            .debug_selector(|| "smoke_idle_tab_content".to_string())
                            .child("Two"),
                    )
                    .render(theme)
                    .debug_selector(|| "smoke_idle_tab".to_string()),
            )
            .render();

        let content = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(components::panel(theme, "Tabs", None, tabs))
            .child(components::panel(
                theme,
                "Input",
                None,
                div()
                    .id("smoke_input")
                    .debug_selector(|| "smoke_input".to_string())
                    .child(self.input.clone()),
            ))
            .child(components::panel(
                theme,
                "Buttons",
                None,
                div()
                    .flex()
                    .gap_2()
                    .child(
                        components::Button::new("b1", "Primary")
                            .style(components::ButtonStyle::Filled)
                            .render(theme, ui_scale::DEFAULT_UI_SCALE_PERCENT),
                    )
                    .child(
                        components::Button::new("b2", "Secondary")
                            .style(components::ButtonStyle::Outlined)
                            .render(theme, ui_scale::DEFAULT_UI_SCALE_PERCENT),
                    ),
            ))
            .into_any_element();

        view::window_frame(
            theme,
            window.window_decorations(),
            content,
            None,
            ui_scale::DEFAULT_UI_SCALE_PERCENT,
        )
    }
}

/// Hold backend operations until the test has observed their loading UI.
/// The guard releases workers even if an assertion panics.
#[derive(Default)]
struct TestOperationGate {
    state: Mutex<(bool, bool)>, // held, started
    released: Condvar,
}

struct PendingTestOperation(Arc<TestOperationGate>);

impl TestOperationGate {
    fn hold(self: &Arc<Self>) -> PendingTestOperation {
        let mut state = self.state.lock().unwrap();
        assert!(!state.0, "an operation is already pending");
        *state = (true, false);
        PendingTestOperation(self.clone())
    }

    fn wait(&self) {
        let mut state = self.state.lock().unwrap();
        if state.0 {
            state.1 = true;
            let _guard = self.released.wait_while(state, |state| state.0).unwrap();
        }
    }
}

impl PendingTestOperation {
    fn has_started(&self) -> bool {
        self.0.state.lock().unwrap().1
    }
}

impl Drop for PendingTestOperation {
    fn drop(&mut self) {
        self.0.state.lock().unwrap().0 = false;
        self.0.released.notify_all();
    }
}

#[derive(Default)]
struct SubmoduleTestBackend {
    gate: Arc<TestOperationGate>,
}

impl GitBackend for SubmoduleTestBackend {
    fn open(&self, workdir: &Path) -> Result<Arc<dyn GitRepository>> {
        Ok(Arc::new(SubmoduleTestRepo {
            gate: self.gate.clone(),
            spec: RepoSpec {
                workdir: workdir.to_path_buf(),
            },
        }))
    }
}

struct SubmoduleTestRepo {
    spec: RepoSpec,
    gate: Arc<TestOperationGate>,
}

impl SubmoduleTestRepo {
    fn unsupported<T>() -> Result<T> {
        Err(Error::new(ErrorKind::Unsupported(
            "Submodule test repo does not implement this operation",
        )))
    }
}

impl GitRepository for SubmoduleTestRepo {
    fn spec(&self) -> &RepoSpec {
        &self.spec
    }

    fn log_head_page(
        &self,
        _limit: usize,
        _cursor: Option<&LogCursor>,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<std::sync::Arc<LogPage>> {
        cancellation.check_cancelled()?;
        let result: Result<std::sync::Arc<LogPage>> = { Self::unsupported() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
        Self::unsupported()
    }

    fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
        Self::unsupported()
    }

    fn current_branch(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<String> {
        cancellation.check_cancelled()?;
        let result: Result<String> = { Self::unsupported() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn list_branches(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<Branch>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<Branch>> = { Self::unsupported() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn list_remotes(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<Remote>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<Remote>> = { Self::unsupported() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn list_remote_branches(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<RemoteBranch>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<RemoteBranch>> = { Self::unsupported() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn status(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<RepoStatus> {
        cancellation.check_cancelled()?;
        let result: Result<RepoStatus> = { Self::unsupported() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
        Self::unsupported()
    }

    fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
        Self::unsupported()
    }

    fn delete_branch(&self, _name: &str) -> Result<()> {
        Self::unsupported()
    }

    fn checkout_branch(&self, _name: &str) -> Result<()> {
        Self::unsupported()
    }

    fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
        Self::unsupported()
    }

    fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
        Self::unsupported()
    }

    /// Keep removal in flight until the spinner test releases it.
    fn remove_worktree_with_output(
        &self,
        _path: &Path,
    ) -> Result<gitcomet_core::services::CommandOutput> {
        self.gate.wait();
        Self::unsupported()
    }

    fn list_submodules(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<Submodule>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<Submodule>> = {
            self.gate.wait();
            Ok(Vec::new())
        };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
        Self::unsupported()
    }

    fn stash_list(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<StashEntry>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<StashEntry>> = { Self::unsupported() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn stash_apply(&self, _index: usize) -> Result<()> {
        Self::unsupported()
    }

    fn stash_drop(&self, _index: usize) -> Result<()> {
        Self::unsupported()
    }

    fn stage(&self, _paths: &[&Path]) -> Result<()> {
        Self::unsupported()
    }

    fn unstage(&self, _paths: &[&Path]) -> Result<()> {
        Self::unsupported()
    }

    fn commit(&self, _message: &str) -> Result<()> {
        Self::unsupported()
    }

    fn fetch_all(&self, _prune: bool) -> Result<gitcomet_core::services::CommandOutput> {
        let result: Result<()> = { Self::unsupported() };
        result.map(|()| gitcomet_core::services::CommandOutput::empty_success("git fetch --all"))
    }

    fn pull(
        &self,
        _mode: PullMode,
        _prune: bool,
    ) -> Result<gitcomet_core::services::CommandOutput> {
        let result: Result<()> = { Self::unsupported() };
        result.map(|()| gitcomet_core::services::CommandOutput::empty_success("git pull"))
    }

    fn push(&self) -> Result<()> {
        Self::unsupported()
    }

    fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
        Self::unsupported()
    }
}

#[derive(Default)]
struct StashTestBackend {
    gate: Arc<TestOperationGate>,
}

impl GitBackend for StashTestBackend {
    fn open(&self, workdir: &Path) -> Result<Arc<dyn GitRepository>> {
        Ok(Arc::new(StashTestRepo {
            gate: self.gate.clone(),
            spec: RepoSpec {
                workdir: workdir.to_path_buf(),
            },
        }))
    }
}

struct StashTestRepo {
    spec: RepoSpec,
    gate: Arc<TestOperationGate>,
}

impl StashTestRepo {
    fn unsupported<T>() -> Result<T> {
        Err(Error::new(ErrorKind::Unsupported(
            "Stash test repo does not implement this operation",
        )))
    }
}

impl GitRepository for StashTestRepo {
    fn spec(&self) -> &RepoSpec {
        &self.spec
    }

    fn log_head_page(
        &self,
        _limit: usize,
        _cursor: Option<&LogCursor>,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<std::sync::Arc<LogPage>> {
        cancellation.check_cancelled()?;
        let result: Result<std::sync::Arc<LogPage>> = { Self::unsupported() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
        Self::unsupported()
    }

    fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
        Self::unsupported()
    }

    fn current_branch(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<String> {
        cancellation.check_cancelled()?;
        let result: Result<String> = { Self::unsupported() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn list_branches(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<Branch>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<Branch>> = { Self::unsupported() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn list_remotes(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<Remote>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<Remote>> = { Self::unsupported() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn list_remote_branches(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<RemoteBranch>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<RemoteBranch>> = { Self::unsupported() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn status(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<RepoStatus> {
        cancellation.check_cancelled()?;
        let result: Result<RepoStatus> = { Self::unsupported() };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
        Self::unsupported()
    }

    fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
        Self::unsupported()
    }

    fn delete_branch(&self, _name: &str) -> Result<()> {
        Self::unsupported()
    }

    fn checkout_branch(&self, _name: &str) -> Result<()> {
        Self::unsupported()
    }

    fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
        Self::unsupported()
    }

    fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
        Self::unsupported()
    }

    fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
        Self::unsupported()
    }

    fn stash_list(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<StashEntry>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<StashEntry>> = {
            self.gate.wait();
            Ok(Vec::new())
        };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn stash_apply(&self, _index: usize) -> Result<()> {
        Self::unsupported()
    }

    fn stash_drop(&self, _index: usize) -> Result<()> {
        Self::unsupported()
    }

    fn stage(&self, _paths: &[&Path]) -> Result<()> {
        Self::unsupported()
    }

    fn unstage(&self, _paths: &[&Path]) -> Result<()> {
        Self::unsupported()
    }

    fn commit(&self, _message: &str) -> Result<()> {
        Self::unsupported()
    }

    fn fetch_all(&self, _prune: bool) -> Result<gitcomet_core::services::CommandOutput> {
        let result: Result<()> = { Self::unsupported() };
        result.map(|()| gitcomet_core::services::CommandOutput::empty_success("git fetch --all"))
    }

    fn pull(
        &self,
        _mode: PullMode,
        _prune: bool,
    ) -> Result<gitcomet_core::services::CommandOutput> {
        let result: Result<()> = { Self::unsupported() };
        result.map(|()| gitcomet_core::services::CommandOutput::empty_success("git pull"))
    }

    fn push(&self) -> Result<()> {
        Self::unsupported()
    }

    fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
        Self::unsupported()
    }
}

fn repo_tab_selector(repo_id: RepoId) -> &'static str {
    Box::leak(format!("repo_tab_{}", repo_id.0).into_boxed_str())
}

fn debug_selector(prefix: &str, ix: usize) -> &'static str {
    Box::leak(format!("{prefix}_{ix}").into_boxed_str())
}

fn wait_for_repo_count(store: &AppStore, expected: usize) -> Arc<gitcomet_state::model::AppState> {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let state = store.snapshot();
        if state.repos.len() == expected {
            return state;
        }
        if Instant::now() >= deadline {
            panic!(
                "timed out waiting for store repos len {expected}, got {}",
                state.repos.len()
            );
        }
        std::thread::yield_now();
    }
}

fn wait_for_repo_open(store: &AppStore, repo_id: RepoId) {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let state = store.snapshot();
        if state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .is_some_and(|repo| matches!(repo.open, Loadable::Ready(())))
        {
            return;
        }
        if Instant::now() >= deadline {
            panic!("timed out waiting for repo {repo_id:?} to open");
        }
        std::thread::yield_now();
    }
}

/// Hand-fed sidebar results must land after startup loads have settled; another
/// queued backend refresh could otherwise replace the fixture.
fn wait_for_sidebar_loads_to_settle(
    cx: &mut gpui::VisualTestContext,
    store: &AppStore,
    view: &gpui::Entity<crate::view::GitCometView>,
    repo_id: RepoId,
) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        sync_view_for_tests(cx, view);

        let state = store.snapshot();
        let repo = state.repos.iter().find(|repo| repo.id == repo_id);
        let settled = repo.is_some_and(|repo| {
            repo.sidebar_data_request.worktrees
                && !matches!(repo.worktrees, Loadable::Loading | Loadable::NotLoaded)
                && !matches!(repo.branches, Loadable::Loading | Loadable::NotLoaded)
                && !repo
                    .loads_in_flight
                    .is_in_flight(gitcomet_state::model::LoadKind::Worktrees)
                && !repo.loads_in_flight.worktree_dirty_in_flight()
                && !repo
                    .loads_in_flight
                    .is_in_flight(gitcomet_state::model::LoadKind::Branches)
        });
        if settled {
            return;
        }

        if Instant::now() >= deadline {
            panic!(
                "timed out waiting for branch and worktree loads to settle: {:?}",
                repo.map(|repo| (
                    repo.sidebar_data_request.worktrees,
                    &repo.branches,
                    &repo.worktrees
                ))
            );
        }

        cx.run_until_parked();
        std::thread::yield_now();
    }
}

fn publish_worktrees_for_test(store: &AppStore, repo_id: RepoId, worktrees: Vec<Worktree>) {
    store.dispatch(Msg::Internal(
        gitcomet_state::msg::InternalMsg::WorktreesLoaded {
            repo_id,
            read: gitcomet_state::msg::WorktreeListRead::observation_for_test(),
            result: Ok(Arc::new(worktrees)),
        },
    ));
}

fn wait_until(description: &str, ready: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        if ready() {
            return;
        }
        if Instant::now() >= deadline {
            panic!("timed out waiting for {description}");
        }
        std::thread::yield_now();
    }
}

fn seed_worktree_repo(
    cx: &mut gpui::VisualTestContext,
    store: &AppStore,
    view: gpui::Entity<crate::view::GitCometView>,
    path: PathBuf,
) {
    store.dispatch(Msg::OpenRepo(path));

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        cx.update(|window, app| {
            view.update(app, |this, cx| {
                crate::view::test_support::sync_store_snapshot(this, cx)
            });
            let _ = window.draw(app);
        });
        cx.run_until_parked();

        let ready = cx.update(|_window, app| !view.read(app).blocks_non_repository_actions());
        if ready {
            return;
        }

        if Instant::now() >= deadline {
            panic!("timed out waiting for the window to leave the Home screen");
        }

        std::thread::sleep(Duration::from_millis(10));
    }
}

fn sync_view_for_tests(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<crate::view::GitCometView>,
) {
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            crate::view::test_support::sync_store_snapshot(this, cx)
        });
        let _ = window.draw(app);
    });
}

fn find_debug_index(
    cx: &mut gpui::VisualTestContext,
    prefix: &str,
    max_ix: usize,
) -> Option<usize> {
    (0..max_ix).find(|ix| cx.debug_bounds(debug_selector(prefix, *ix)).is_some())
}

fn wait_for_debug_index(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<crate::view::GitCometView>,
    prefix: &str,
    max_ix: usize,
) -> usize {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        sync_view_for_tests(cx, view);

        if let Some(ix) = find_debug_index(cx, prefix, max_ix) {
            return ix;
        }

        if Instant::now() >= deadline {
            panic!("timed out waiting for debug selector prefix {prefix}");
        }

        cx.run_until_parked();
        std::thread::yield_now();
    }
}

fn click_debug_selector(
    cx: &mut gpui::VisualTestContext,
    selector: &'static str,
    click_count: usize,
) {
    click_debug_selector_with_button(cx, selector, MouseButton::Left, click_count);
}

fn click_debug_selector_with_button(
    cx: &mut gpui::VisualTestContext,
    selector: &'static str,
    button: MouseButton,
    click_count: usize,
) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("expected debug selector {selector}"));
    let center = bounds.center();
    cx.simulate_mouse_move(center, None, Modifiers::default());
    cx.simulate_event(MouseDownEvent {
        position: center,
        modifiers: Modifiers::default(),
        button,
        click_count,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position: center,
        modifiers: Modifiers::default(),
        button,
        click_count,
    });
}

fn restore_session_and_draw(
    cx: &mut gpui::VisualTestContext,
    store: &AppStore,
    view: gpui::Entity<crate::view::GitCometView>,
    repos: Vec<PathBuf>,
) -> Vec<RepoId> {
    for repo in repos.iter() {
        fs::create_dir_all(repo).unwrap_or_else(|error| {
            panic!("failed to create test repo dir {}: {error}", repo.display())
        });
    }

    store.dispatch(Msg::RestoreSession {
        open_repos: repos.clone(),
        active_repo: repos.first().cloned(),
    });

    let state = wait_for_repo_count(store, repos.len());
    let ids = state.repos.iter().map(|r| r.id).collect::<Vec<_>>();
    let selectors = ids
        .iter()
        .copied()
        .map(repo_tab_selector)
        .collect::<Vec<_>>();

    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        sync_view_for_tests(cx, &view);

        if selectors
            .iter()
            .all(|selector| cx.debug_bounds(selector).is_some())
        {
            return ids;
        }

        if Instant::now() >= deadline {
            panic!(
                "timed out waiting for repo tabs to render; missing selectors: {:?}",
                selectors
                    .into_iter()
                    .filter(|selector| cx.debug_bounds(selector).is_none())
                    .collect::<Vec<_>>()
            );
        }

        cx.run_until_parked();
        std::thread::yield_now();
    }
}

struct PanelLayoutTestView {
    theme: AppTheme,
    handle: gpui::UniformListScrollHandle,
}

impl PanelLayoutTestView {
    fn new() -> Self {
        Self {
            theme: AppTheme::gitcomet_dark(),
            handle: gpui::UniformListScrollHandle::default(),
        }
    }
}

impl gpui::Render for PanelLayoutTestView {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        let theme = self.theme;

        let header = div().id("diff_header").h(px(24.0)).child("Header");
        let list = gpui::uniform_list(
            "diff_list",
            200,
            cx.processor(
                |_this: &mut PanelLayoutTestView,
                 range: std::ops::Range<usize>,
                 _window: &mut gpui::Window,
                 _cx: &mut gpui::Context<PanelLayoutTestView>| {
                    range
                        .map(|ix| {
                            div()
                                .id(ix)
                                .h(px(20.0))
                                .px_2()
                                .child(format!("Row {ix}"))
                                .into_any_element()
                        })
                        .collect::<Vec<_>>()
                },
            ),
        )
        .h_full()
        .track_scroll(&self.handle);

        let body = div()
            .id("diff_body")
            .debug_selector(|| "diff_body".to_string())
            .flex()
            .flex_col()
            .h_full()
            .child(header)
            .child({
                let scrollbar =
                    components::Scrollbar::new("diff_scrollbar_test", self.handle.clone());
                #[cfg(test)]
                let scrollbar = scrollbar.debug_selector("diff_scrollbar_test");

                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .relative()
                    .child(list)
                    .child(scrollbar.render(theme))
            });

        div().size_full().bg(theme.colors.surface.canvas).child(
            components::panel(theme, "Panel", None, body)
                .flex_1()
                .h_full(),
        )
    }
}

/// Opens a window with `count` repositories and returns their ids in tab
/// order. Enough of them to overflow the strip is the point of most callers.
fn open_repo_tabs(
    cx: &mut gpui::VisualTestContext,
    store: &AppStore,
    view: gpui::Entity<crate::view::GitCometView>,
    label: &str,
    count: usize,
) -> Vec<RepoId> {
    let base =
        std::env::temp_dir().join(format!("gitcomet_ui_test_{label}_{}", std::process::id()));
    let repos: Vec<PathBuf> = (0..count)
        .map(|ix| base.join(format!("repository-number-{ix}")))
        .collect();
    restore_session_and_draw(cx, store, view, repos)
}

fn repo_tab_scroll(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<crate::view::GitCometView>,
) -> (Pixels, Pixels) {
    cx.update(|_window, app| crate::view::test_support::repo_tab_scroll(view.read(app), app))
}

mod component_rendering;
mod menus;
mod pointer_gestures;
mod repository_tabs;
mod scrollbars;
mod sidebar_loading;
mod text_input;
mod worktree_badges;
