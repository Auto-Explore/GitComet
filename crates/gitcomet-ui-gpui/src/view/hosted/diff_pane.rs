use super::*;
use crate::view::hosted::environment::DiffPaneEnvironment;
use crate::view::panes::diff::DiffRendererInit;
use crate::view::panes::diff::diff_cache::SharedFileDiffCache;
use crate::view::panes::diff::input::DiffPaneInput;
use gitcomet_core::domain::DiffTarget;
use gitcomet_core::text_format::TextEncoding;
use gitcomet_extension_api::{
    DiffAnnotations, DiffInset, DiffLayout, DiffLegendItem, DiffLineRange, DiffLineSide,
    DiffPaneEvent, DiffPaneOptions, DiffPanePolicy, DiffScrollAnchor, DiffSnapshot,
    RepositoryHandle, StateSubscription, WindowHost, panes::DiffPaneImpl,
};
use gitcomet_state::diff_session::{DiffSession, DiffSessionMsg, DiffViewId};
use std::rc::Rc;

enum PaneSource {
    Session {
        store: std::sync::Weak<AppStore>,
        repository: RepositoryHandle,
        target: DiffTarget,
    },
    Snapshot(DiffSnapshot),
}

static ENGAGED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
fn next_engaged() -> u64 {
    ENGAGED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1
}

pub(crate) struct DiffPaneView {
    host: WindowHost,
    environment: DiffPaneEnvironment,
    engaged: u64,
    _focus_in: Option<gpui::Subscription>,
    focus_window: Option<gpui::WindowId>,
    renderer: Option<Entity<DiffRenderer>>,
    renderer_subscription: Option<gpui::Subscription>,
    source: PaneSource,
    view_id: DiffViewId,
    options: DiffPaneOptions,
    file_cache: Option<Arc<SharedFileDiffCache>>,
    raw: [Option<super::raw_lines::RawLines>; 2],
    raw_task: Option<gpui::Task<()>>,
    revision: u64,
    loaded_revision: Option<u64>,
    session_revision: Option<u64>,
    snapshot_rev: u64,
    loading: bool,
    pending_target: Option<DiffTarget>,
    error: Option<SharedString>,
    insets: Arc<[DiffInset]>,
    annotations: Arc<DiffAnnotations>,
    legend: Vec<DiffLegendItem>,
    selection: Option<DiffLineRange>,
    search: SharedString,
    pending_reveal: Option<(DiffLineSide, u32)>,
    pending_scroll_top: bool,
    _state: Option<StateSubscription>,
}

impl DiffPaneView {
    fn with_source(
        host: WindowHost,
        environment: DiffPaneEnvironment,
        source: PaneSource,
        mut options: DiffPaneOptions,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        if matches!(source, PaneSource::Snapshot(_)) {
            read_only_policy(&mut options.policy);
        }
        let window = environment.window;
        let weak = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| {
                let _ = weak.update(cx, |pane, cx| pane.prepare_renderer(window, cx));
            });
        });
        Self {
            host,
            environment,
            source,
            options,
            view_id: DiffViewId::next(),
            engaged: next_engaged(),
            _focus_in: None,
            focus_window: None,
            renderer: None,
            renderer_subscription: None,
            file_cache: None,
            raw: [None, None],
            raw_task: None,
            revision: 0,
            loaded_revision: None,
            session_revision: None,
            snapshot_rev: 0,
            loading: true,
            pending_target: None,
            error: None,
            insets: Arc::from([]),
            annotations: Arc::default(),
            legend: Vec::new(),
            selection: None,
            search: SharedString::default(),
            pending_reveal: None,
            pending_scroll_top: false,
            _state: None,
        }
    }

    pub(in crate::view) fn new(
        host: WindowHost,
        environment: DiffPaneEnvironment,
        store: std::sync::Weak<AppStore>,
        repository: RepositoryHandle,
        target: DiffTarget,
        options: DiffPaneOptions,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let weak = cx.weak_entity();
        let subscription = host
            .observe_state(move |_, cx| {
                let _ = weak.update(cx, |pane, cx| pane.sync(cx));
            })
            .ok();
        let mut pane = Self::with_source(
            host,
            environment,
            PaneSource::Session {
                store,
                repository,
                target: target.clone(),
            },
            options,
            cx,
        );
        pane._state = subscription;
        pane.open(target);
        pane
    }

    pub(in crate::view) fn snapshot(
        host: WindowHost,
        environment: DiffPaneEnvironment,
        snapshot: DiffSnapshot,
        options: DiffPaneOptions,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let mut pane = Self::with_source(
            host,
            environment,
            PaneSource::Snapshot(snapshot.clone()),
            options,
            cx,
        );
        pane.set_snapshot(snapshot, cx);
        pane
    }

    fn preferred_view_mode(&self, cx: &App) -> DiffViewMode {
        self.environment.preferences.read(cx).diff.view_mode
    }

    fn binding(&self) -> Option<crate::view::pane_store::DiffBinding> {
        let PaneSource::Session { repository, .. } = &self.source else {
            return None;
        };
        Some(crate::view::pane_store::DiffBinding {
            repo_id: repository.repo_id(),
            lifetime: repository.lifetime(),
            view: self.view_id,
        })
    }

    fn input(&self, cx: &App) -> Option<DiffPaneInput> {
        match &self.source {
            PaneSource::Session { .. } => Some(DiffPaneInput::session(
                self.host.state(cx).ok()?,
                self.binding()?,
            )),
            PaneSource::Snapshot(snapshot) => {
                let target = snapshot
                    .patch
                    .as_ref()
                    .map(|patch| patch.target.clone())
                    .unwrap_or_else(|| {
                        DiffTarget::working_tree(snapshot.path.clone(), DiffArea::Unstaged)
                    });
                let state = gitcomet_state::model::DiffState {
                    diff_target: Some(target),
                    diff_target_rev: self.snapshot_rev,
                    diff_state_rev: self.snapshot_rev,
                    diff_rev: self.snapshot_rev,
                    diff_file_rev: self.snapshot_rev,
                    diff: snapshot
                        .patch
                        .clone()
                        .map(Loadable::Ready)
                        .unwrap_or(Loadable::NotLoaded),
                    diff_file: Loadable::Ready(
                        self.file_cache.as_ref().map(|cache| cache.file.clone()),
                    ),
                    ..Default::default()
                };
                Some(DiffPaneInput::Snapshot {
                    state: Arc::new(state),
                    path: snapshot.path.clone(),
                })
            }
        }
    }

    fn prepare_renderer(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        if self.renderer.is_none() {
            let Some(input) = self.input(cx) else {
                return;
            };
            let mut services = self.environment.services.clone();
            if let Some(binding) = self.binding() {
                services.store.bind(binding, self.options.policy);
                if let Some(state) = input.repository_state() {
                    services
                        .store
                        .set_linked(services.store.shows_linked_worktree(state));
                }
            } else {
                services.store.disable_commands();
            }
            let mut preferences = self.environment.preferences.read(cx);
            preferences.diff.content_mode = DiffContentMode::Full;
            let preferred = preferences.diff.view_mode;
            let theme = self.host.theme(cx);
            let renderer = cx.new(|cx| {
                let mut pane = DiffRenderer::new(
                    input,
                    &preferences,
                    services,
                    DiffRendererInit {
                        theme,
                        view_mode: GitCometViewMode::Normal,
                        focused_mergetool_labels: None,
                        focused_mergetool_exit_code: None,
                    },
                    window,
                    cx,
                );
                pane.set_hosted_decor(
                    self.view_id.0,
                    self.options.clone(),
                    self.annotations.clone(),
                    self.insets.clone(),
                );
                pane.hosted_decor.as_mut().unwrap().file_cache = self.file_cache.clone();
                pane.annotate_enabled = self.options.policy.blame;
                pane.diff_view = diff_view_mode(self.options.layout, preferred);
                pane.hosted_content_width = Some(
                    self.options
                        .content_width
                        .map(px)
                        .unwrap_or(window.viewport_size().width),
                );
                pane.diff_autoscroll_pending = self.pending_reveal.is_none();
                pane.ensure_file_diff_cache(cx);
                pane
            });
            self.renderer_subscription = Some(cx.observe(&renderer, |this, renderer, cx| {
                let selection = renderer.read(cx).hosted_selection();
                if this.selection != selection {
                    this.selection = selection;
                    this.emit(DiffPaneEvent::SelectionChanged(selection), cx);
                }
                if let Some((side, line)) = this.pending_reveal
                    && renderer.update(cx, |pane, _| {
                        pane.hosted_reveal_at(side, line, this.pending_scroll_top)
                    })
                {
                    this.pending_reveal = None;
                }
                this.notify_loaded(cx);
                cx.notify();
            }));
            self.renderer = Some(renderer);
            self.set_search(self.search.clone(), cx);
            self.notify_loaded(cx);
        }
        if self.focus_window != Some(window.window_handle().window_id()) {
            let focus = self
                .renderer
                .as_ref()
                .unwrap()
                .read(cx)
                .diff_panel_focus_handle
                .clone();
            self._focus_in = Some(cx.on_focus_in(&focus, window, |this, _, _| {
                this.engaged = next_engaged();
            }));
            self.focus_window = Some(window.window_handle().window_id());
        }
    }

    fn notify_loaded(&mut self, cx: &mut gpui::Context<Self>) {
        if self.loading || self.raw_task.is_some() || self.loaded_revision == Some(self.revision) {
            return;
        }
        let Some(renderer) = &self.renderer else {
            return;
        };
        let pane = renderer.read(cx);
        if pane.file_diff_cache_inflight.is_some() {
            return;
        }
        if let Some(error) = &pane.file_diff_cache_error {
            let error: SharedString = error.to_string().into();
            if self.error.as_ref() != Some(&error) {
                self.error = Some(error.clone());
                self.emit(DiffPaneEvent::Error(error), cx);
            }
            return;
        }
        self.loaded_revision = Some(self.revision);
        self.emit(DiffPaneEvent::Loaded, cx);
    }

    fn sync_renderer(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(input) = self.input(cx) else {
            return;
        };
        if let Some(renderer) = &self.renderer {
            renderer.update(cx, |pane, cx| {
                if let Some(state) = input.repository_state() {
                    pane.store
                        .set_linked(pane.store.shows_linked_worktree(state));
                }
                pane.apply_input(input, cx);
            });
        }
    }

    fn reset(&mut self) {
        self.engaged = next_engaged();
        self.revision = self.revision.wrapping_add(1);
        self.file_cache = None;
        self.raw = [None, None];
        self.raw_task = None;
        self.loading = true;
        self.error = None;
        self.selection = None;
        self.pending_reveal = None;
    }

    fn set_file_cache(
        &mut self,
        cache: Option<Arc<SharedFileDiffCache>>,
        cx: &mut gpui::Context<Self>,
    ) {
        let changed = match (&self.file_cache, &cache) {
            (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
            (None, None) => false,
            _ => true,
        };
        self.file_cache = cache;
        if let Some(renderer) = &self.renderer {
            renderer.update(cx, |pane, _| {
                pane.hosted_decor.as_mut().unwrap().file_cache = self.file_cache.clone();
            });
        }
        if !changed {
            return;
        }
        self.raw_task = None;
        self.raw = [None, None];
        if let Some(cache) = &self.file_cache {
            let cache = cache.clone();
            let file = cache.file.clone();
            self.raw_task = Some(cx.spawn(async move |pane, cx| {
                let raw = cx
                    .background_executor()
                    .spawn(async move { super::raw_lines::RawLines::file(&file) })
                    .await;
                let _ = pane.update(cx, |pane, cx| {
                    if !pane
                        .file_cache
                        .as_ref()
                        .is_some_and(|shown| Arc::ptr_eq(shown, &cache))
                    {
                        return;
                    }
                    pane.raw = raw;
                    pane.raw_task = None;
                    pane.notify_loaded(cx);
                    cx.notify();
                });
            }));
        }
    }

    pub(crate) fn set_snapshot(&mut self, snapshot: DiffSnapshot, cx: &mut gpui::Context<Self>) {
        let PaneSource::Snapshot(shown) = &mut self.source else {
            return;
        };
        *shown = snapshot.clone();
        self.snapshot_rev = self.snapshot_rev.wrapping_add(1);
        self.reset();
        let cache = snapshot.patch.is_none().then(|| {
            Arc::new(SharedFileDiffCache::new(
                Arc::new(gitcomet_core::domain::FileDiffText::new_shared(
                    snapshot.path.clone(),
                    Some(snapshot.old),
                    Some(snapshot.new),
                )),
                None,
                std::path::PathBuf::new(),
            ))
        });
        self.set_file_cache(cache, cx);
        self.loading = false;
        self.sync_renderer(cx);
        self.notify_loaded(cx);
        cx.notify();
    }

    fn sync(&mut self, cx: &mut gpui::Context<Self>) {
        let Ok(state) = self.host.state(cx) else {
            return;
        };
        let Some(session) = self.session(&state) else {
            return;
        };
        self.pending_target = None;
        if let PaneSource::Session { target, .. } = &mut self.source {
            *target = session.target.clone();
        }
        if self.session_revision == Some(session.rev) {
            self.sync_renderer(cx);
            return;
        }
        self.revision = self.revision.wrapping_add(1);
        self.session_revision = Some(session.rev);
        let file = match &session.diff_file {
            Loadable::Ready(Some(file)) => Some(file.clone()),
            _ => None,
        };
        let patch = match &session.diff {
            Loadable::Ready(patch) => Some(patch.clone()),
            _ => None,
        };
        let workdir = session
            .worktree()
            .map(std::path::Path::to_path_buf)
            .or_else(|| {
                self.binding().and_then(|binding| {
                    state
                        .repos
                        .iter()
                        .find(|repo| repo.id == binding.repo_id)
                        .map(|repo| repo.spec.workdir.clone())
                })
            });
        let cache = file.zip(workdir).map(|(file, workdir)| {
            self.file_cache
                .as_ref()
                .filter(|cache| cache.matches(&file, patch.as_ref(), &workdir))
                .cloned()
                .unwrap_or_else(|| {
                    Arc::new(SharedFileDiffCache::new(file, patch.as_ref(), workdir))
                })
        });
        self.set_file_cache(cache, cx);
        let error = match (&session.diff, &session.diff_file) {
            (Loadable::Error(error), _) | (_, Loadable::Error(error)) => {
                Some(SharedString::from(error.clone()))
            }
            _ => None,
        };
        if let Some(error) = &error
            && self.error.as_ref() != Some(error)
        {
            self.emit(DiffPaneEvent::Error(error.clone()), cx);
        }
        self.error = error;
        self.loading = session.is_loading();
        if session.diff_target.is_none() {
            self.emit(DiffPaneEvent::TargetChanged(None), cx);
        }
        self.sync_renderer(cx);
        self.notify_loaded(cx);
        cx.notify();
    }

    pub(crate) fn set_target(&mut self, target: DiffTarget, cx: &mut gpui::Context<Self>) {
        if matches!(self.source, PaneSource::Snapshot(_)) {
            return;
        }
        self.open(target.clone());
        if let Some(renderer) = &self.renderer {
            renderer.update(cx, |pane, cx| pane.set_pending_target(target.clone(), cx));
        }
        self.emit(DiffPaneEvent::TargetChanged(Some(target)), cx);
        cx.notify();
    }

    fn reveal_at(
        &mut self,
        side: DiffLineSide,
        line: u32,
        top: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        self.pending_scroll_top = top;
        self.pending_reveal = Some((side, line));
        if let Some(renderer) = &self.renderer
            && renderer.update(cx, |pane, cx| {
                pane.diff_autoscroll_pending = false;
                let found = pane.hosted_reveal_at(side, line, top);
                cx.notify();
                found
            })
        {
            self.pending_reveal = None;
        }
    }

    pub(crate) fn set_search(&mut self, query: SharedString, cx: &mut gpui::Context<Self>) {
        if !self.options.policy.search {
            return;
        }
        self.search = query.clone();
        if let Some(renderer) = &self.renderer {
            renderer.update(cx, |pane, cx| {
                pane.diff_search_query = query.clone();
                pane.diff_search_active = !query.is_empty();
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text(query.to_string(), cx));
                pane.diff_search_recompute_matches_and_scroll_to_first();
                cx.notify();
            });
        }
        cx.notify();
    }

    pub(crate) fn set_annotations(
        &mut self,
        annotations: Arc<DiffAnnotations>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.annotations = annotations;
        self.sync_decor(cx);
        cx.notify();
    }
    pub(crate) fn set_insets(&mut self, insets: Vec<DiffInset>, cx: &mut gpui::Context<Self>) {
        self.insets = insets.into();
        self.sync_decor(cx);
        cx.notify();
    }
    pub(in crate::view) fn renderer(&self) -> Option<&Entity<DiffRenderer>> {
        self.renderer.as_ref()
    }

    pub(in crate::view) fn engaged(&self) -> u64 {
        self.engaged
    }

    fn emit(&self, event: DiffPaneEvent, cx: &mut gpui::Context<Self>) {
        if let Some(handler) = &self.options.on_event {
            let handler = handler.clone();
            cx.defer(move |cx| handler(event, cx));
        }
    }

    fn sync_decor(&self, cx: &mut gpui::Context<Self>) {
        if let Some(renderer) = &self.renderer {
            renderer.update(cx, |pane, cx| {
                pane.set_hosted_decor(
                    self.view_id.0,
                    self.options.clone(),
                    self.annotations.clone(),
                    self.insets.clone(),
                );
                cx.notify();
            });
        }
    }

    pub(crate) fn set_legend(&mut self, legend: Vec<DiffLegendItem>, cx: &mut gpui::Context<Self>) {
        self.legend = legend;
        cx.notify();
    }

    fn open(&mut self, target: DiffTarget) {
        if !matches!(self.source, PaneSource::Session { .. }) {
            return;
        }
        self.reset();
        self.pending_target = Some(target.clone());
        let PaneSource::Session {
            store,
            repository,
            target: shown,
        } = &mut self.source
        else {
            return;
        };
        *shown = target.clone();
        let Some(store) = store.upgrade() else {
            return;
        };
        store.dispatch(Msg::DiffSession(DiffSessionMsg::Open {
            repo_id: repository.repo_id(),
            lifetime: repository.lifetime(),
            view: self.view_id,
            target,
        }));
        if self.options.policy.blame {
            store.dispatch(Msg::DiffSession(DiffSessionMsg::LoadBlame {
                repo_id: repository.repo_id(),
                lifetime: repository.lifetime(),
                view: self.view_id,
            }));
        }
    }

    fn session<'a>(&self, state: &'a AppState) -> Option<&'a DiffSession> {
        let PaneSource::Session { repository, .. } = &self.source else {
            return None;
        };
        state
            .repos
            .iter()
            .find(|repo| {
                repo.id == repository.repo_id() && repo.lifetime() == repository.lifetime()
            })?
            .diff_sessions
            .get(&self.view_id)
            .filter(|session| {
                self.pending_target.as_ref().is_none_or(|target| {
                    session.target == *target
                        && session.target.old_file_path() == target.old_file_path()
                })
            })
    }

    pub(crate) fn set_encoding(&mut self, encoding: Option<TextEncoding>) {
        if let PaneSource::Session {
            store, repository, ..
        } = &self.source
            && let Some(store) = store.upgrade()
        {
            store.dispatch(Msg::DiffSession(DiffSessionMsg::SetEncoding {
                repo_id: repository.repo_id(),
                lifetime: repository.lifetime(),
                view: self.view_id,
                encoding,
            }));
        }
    }

    pub(crate) fn reveal(&mut self, side: DiffLineSide, line: u32, cx: &mut gpui::Context<Self>) {
        self.reveal_at(side, line, false, cx);
    }
    #[cfg(test)]
    pub(in crate::view) fn view_mode_for_test(&self, cx: &App) -> Option<DiffViewMode> {
        self.renderer
            .as_ref()
            .map(|renderer| renderer.read(cx).diff_view)
    }
    #[cfg(test)]
    pub(crate) fn view_id(&self) -> u64 {
        self.view_id.0
    }
    #[cfg(test)]
    pub(crate) fn renderer_offers(&self, cx: &App) -> Option<(bool, bool)> {
        let renderer = self.renderer.as_ref()?.read(cx);
        Some((
            renderer.diff_stage_gutter_area().is_some(),
            renderer.editable_path_for_current_target().is_some(),
        ))
    }
    #[cfg(test)]
    pub(crate) fn renderer_for_test(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<DiffRenderer> {
        self.prepare_renderer(window, cx);
        self.renderer.clone().expect("hosted renderer")
    }
    #[cfg(test)]
    pub(crate) fn renderer_workdir(&self, cx: &App) -> Option<std::path::PathBuf> {
        let renderer = self.renderer.as_ref()?.read(cx);
        renderer
            .rendered_diff_workdir()
            .map(std::path::Path::to_path_buf)
    }
    #[cfg(test)]
    pub(crate) fn renderer_dispatch(&self, msg: Msg, cx: &App) {
        if let Some(renderer) = &self.renderer {
            renderer.read(cx).store.dispatch(msg);
        }
    }
    #[cfg(test)]
    pub(crate) fn shares_renderer_rows(&self, cx: &App) -> bool {
        self.file_cache.as_ref().is_some_and(|cache| {
            self.renderer.as_ref().is_some_and(|renderer| {
                renderer
                    .read(cx)
                    .file_diff_row_provider
                    .as_ref()
                    .is_some_and(|rows| {
                        cache
                            .build()
                            .is_ok_and(|prepared| Arc::ptr_eq(rows, &prepared.row_provider))
                    })
            })
        })
    }
    #[cfg(feature = "benchmarks")]
    pub(in crate::view) fn benchmark_ready(&self, cx: &mut App) -> bool {
        self.renderer
            .as_ref()
            .is_some_and(|pane| pane.update(cx, |pane, _| pane.diff_list_len() >= 400))
    }
    #[cfg(feature = "benchmarks")]
    pub(in crate::view) fn benchmark_window(
        &mut self,
        start: usize,
        count: usize,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> usize {
        self.prepare_renderer(window, cx);
        self.renderer.as_ref().map_or(0, |renderer| {
            renderer.update(cx, |pane, cx| {
                let end = (start + count).min(pane.diff_list_len());
                let rows = DiffRenderer::render_projected_inline(pane, start..end, window, cx);
                std::hint::black_box(rows).len()
            })
        })
    }
    #[cfg(test)]
    pub(crate) fn marker_builds(&self, cx: &App) -> usize {
        self.renderer
            .as_ref()
            .and_then(|renderer| renderer.read(cx).hosted_decor.as_ref())
            .map_or(0, |decor| decor.marker_builds)
    }
}

impl Render for DiffPaneView {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        self.prepare_renderer(window, cx);
        let theme = self.host.theme(cx);
        let ui_scale = ui_scale::UiScale::current(cx);
        let pane_id = self.view_id.0;
        let status = if let Some(error) = &self.error {
            Some(error.clone())
        } else if self
            .renderer
            .as_ref()
            .is_none_or(|renderer| renderer.read(cx).diff_visible_len() == 0)
        {
            Some(if self.loading {
                "Loading…".into()
            } else {
                "No changes".into()
            })
        } else {
            None
        };
        let legend = (!self.legend.is_empty()).then(|| {
            div()
                .id(("hosted_diff_legend", pane_id))
                .debug_selector(move || format!("hosted_diff_{pane_id}_legend"))
                .flex()
                .flex_wrap()
                .flex_none()
                .gap(ui_scale.px(10.0))
                .px(ui_scale.px(8.0))
                .py(ui_scale.px(4.0))
                .text_size(theme.ui_text(12.0))
                .text_color(theme.colors.foreground.secondary)
                .border_b_1()
                .border_color(theme.colors.stroke.subtle)
                .children(self.legend.iter().map(|item| {
                    div()
                        .flex()
                        .items_center()
                        .gap(ui_scale.px(4.0))
                        .child(
                            div()
                                .size(ui_scale.px(8.0))
                                .rounded(ui_scale.px(2.0))
                                .bg(item.color),
                        )
                        .child(item.label.clone())
                }))
        });
        let actions = self
            .selection
            .filter(|_| self.options.policy.select_lines)
            .filter(|_| !self.options.selection_actions.is_empty())
            .map(|range| {
                div()
                    .flex()
                    .flex_none()
                    .gap(ui_scale.px(4.0))
                    .px(ui_scale.px(8.0))
                    .py(ui_scale.px(4.0))
                    .border_b_1()
                    .border_color(theme.colors.stroke.subtle)
                    .children(self.options.selection_actions.iter().enumerate().map(
                        |(index, action)| {
                            let run = Rc::clone(&action.run);
                            components::Button::new(
                                format!("hosted_diff_{pane_id}_action_{index}"),
                                action.label.clone(),
                            )
                            .on_click(theme, cx, move |_, _, _, cx| {
                                let run = Rc::clone(&run);
                                cx.defer(move |cx| run(range, cx));
                            })
                        },
                    ))
            });
        let markers = self
            .renderer
            .as_ref()
            .map(|renderer| renderer.update(cx, |pane, _| pane.hosted_markers()))
            .unwrap_or_default();
        div()
            .id(("hosted_diff_pane", self.view_id.0))
            .debug_selector(move || format!("hosted_diff_{pane_id}"))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.colors.surface.canvas)
            // A pane may be mounted anywhere; its text should not depend on
            // what encloses it.
            .text_color(theme.colors.foreground.primary)
            .children(legend)
            .children(actions)
            .when_some(status, |pane, status| {
                pane.child(
                    div()
                        .p_2()
                        .text_color(theme.colors.foreground.secondary)
                        .child(status),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .relative()
                    .child(if let Some(renderer) = &self.renderer {
                        let measured = renderer.downgrade();
                        let explicit = self.options.content_width;
                        div()
                            .size_full()
                            .relative()
                            .child(renderer.clone())
                            .child(
                                gpui::canvas(
                                    move |bounds, _, cx| {
                                        let width = explicit.map(px).unwrap_or(bounds.size.width);
                                        let _ = measured.update(cx, |pane, cx| {
                                            if pane.hosted_content_width != Some(width) {
                                                pane.hosted_content_width = Some(width);
                                                cx.notify();
                                            }
                                        });
                                    },
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .size_full(),
                            )
                            .into_any_element()
                    } else {
                        div().into_any_element()
                    })
                    .when(!markers.is_empty(), |list| {
                        list.child(
                            div()
                                .debug_selector(move || format!("hosted_diff_{pane_id}_markers"))
                                .absolute()
                                .top_0()
                                .bottom_0()
                                .right_0()
                                .w(ui_scale.px(4.0))
                                .children(markers.iter().map(|(fraction, color)| {
                                    div()
                                        .absolute()
                                        .left_0()
                                        .right_0()
                                        .top(gpui::relative(*fraction))
                                        .h(ui_scale.px(2.0))
                                        .bg(*color)
                                })),
                        )
                    }),
            )
    }
}

impl Drop for DiffPaneView {
    fn drop(&mut self) {
        if let PaneSource::Session {
            store, repository, ..
        } = &self.source
            && let Some(store) = store.upgrade()
        {
            store.dispatch(Msg::DiffSession(DiffSessionMsg::Close {
                repo_id: repository.repo_id(),
                lifetime: repository.lifetime(),
                view: self.view_id,
            }));
        }
    }
}

fn diff_view_mode(layout: DiffLayout, preferred: DiffViewMode) -> DiffViewMode {
    match layout {
        DiffLayout::Split => DiffViewMode::Split,
        DiffLayout::Inline => DiffViewMode::Inline,
        _ => preferred,
    }
}

/// The extension-facing handle; it owns the pane.
pub(crate) struct HostedDiffPane {
    pub(crate) entity: Entity<DiffPaneView>,
}

impl DiffPaneImpl for HostedDiffPane {
    fn view(&self) -> gpui::AnyView {
        self.entity.clone().into()
    }

    fn target(&self, cx: &App) -> Option<DiffTarget> {
        let pane = self.entity.read(cx);
        if let Some(target) = &pane.pending_target {
            return Some(target.clone());
        }
        let state = pane.host.state(cx).ok()?;
        pane.session(&state)?.diff_target.clone()
    }

    fn set_target(&self, target: DiffTarget, cx: &mut App) {
        self.entity
            .update(cx, |pane, cx| pane.set_target(target, cx));
    }

    fn set_snapshot(&self, snapshot: DiffSnapshot, cx: &mut App) {
        self.entity
            .update(cx, |pane, cx| pane.set_snapshot(snapshot, cx));
    }

    fn set_encoding(&self, encoding: Option<TextEncoding>, cx: &mut App) {
        self.entity
            .update(cx, |pane, _| pane.set_encoding(encoding));
    }

    fn is_loading(&self, cx: &App) -> bool {
        let pane = self.entity.read(cx);
        pane.loading
            || pane.raw_task.is_some()
            || pane
                .renderer
                .as_ref()
                .is_none_or(|renderer| renderer.read(cx).file_diff_cache_inflight.is_some())
    }

    fn selection(&self, cx: &App) -> Option<DiffLineRange> {
        let pane = self.entity.read(cx);
        pane.renderer
            .as_ref()
            .and_then(|renderer| renderer.read(cx).hosted_selection())
            .or(pane.selection)
    }

    fn reveal(&self, side: DiffLineSide, line: u32, cx: &mut App) {
        self.entity
            .update(cx, |pane, cx| pane.reveal(side, line, cx));
    }

    fn set_search(&self, query: SharedString, cx: &mut App) {
        self.entity
            .update(cx, |pane, cx| pane.set_search(query, cx));
    }

    fn search_matches(&self, cx: &App) -> usize {
        let pane = self.entity.read(cx);
        pane.renderer.as_ref().map_or(0, |renderer| {
            renderer.read(cx).hosted_search_match_count(&pane.search)
        })
    }

    fn set_annotations(&self, annotations: Arc<DiffAnnotations>, cx: &mut App) {
        self.entity
            .update(cx, |pane, cx| pane.set_annotations(annotations, cx));
    }

    fn set_legend(&self, legend: Vec<DiffLegendItem>, cx: &mut App) {
        self.entity
            .update(cx, |pane, cx| pane.set_legend(legend, cx));
    }

    fn set_insets(&self, insets: Vec<DiffInset>, cx: &mut App) {
        self.entity
            .update(cx, |pane, cx| pane.set_insets(insets, cx));
    }

    fn selected_text(&self, cx: &App) -> Option<String> {
        let pane = self.entity.read(cx);
        let range = self.selection(cx)?;
        Some(pane.renderer.as_ref()?.read(cx).hosted_selected_text(range))
    }
    fn selected_bytes(&self, cx: &App) -> Option<Arc<[u8]>> {
        let range = self.selection(cx)?;
        let pane = self.entity.read(cx);
        pane.raw[match range.side {
            DiffLineSide::Old => 0,
            DiffLineSide::New => 1,
        }]
        .as_ref()?
        .range(range.start, range.end)
    }
    fn set_policy(&self, policy: DiffPanePolicy, cx: &mut App) {
        self.entity.update(cx, |pane, cx| {
            let mut policy = policy;
            if matches!(pane.source, PaneSource::Snapshot(_)) {
                policy.allow_edit = false;
                policy.allow_stage = false;
                policy.allow_annotate = false;
                policy.blame = false;
                policy.file_navigation = false;
            }
            pane.options.policy = policy;
            if let Some(renderer) = &pane.renderer {
                renderer.update(cx, |renderer, cx| {
                    renderer.annotate_enabled = policy.blame;
                    cx.notify();
                });
            }
            if policy.blame
                && let PaneSource::Session {
                    store, repository, ..
                } = &pane.source
                && let Some(store) = store.upgrade()
            {
                store.dispatch(Msg::DiffSession(DiffSessionMsg::LoadBlame {
                    repo_id: repository.repo_id(),
                    lifetime: repository.lifetime(),
                    view: pane.view_id,
                }));
            }
            pane.sync_decor(cx);
            cx.notify();
        });
    }
    fn set_layout(&self, layout: DiffLayout, cx: &mut App) {
        self.entity.update(cx, |pane, cx| {
            pane.options.layout = layout;
            let preferred = pane.preferred_view_mode(cx);
            if let Some(renderer) = &pane.renderer {
                renderer.update(cx, |pane, cx| {
                    pane.set_diff_view_mode(diff_view_mode(layout, preferred), cx)
                });
            }
            cx.notify();
        });
    }
    fn set_content_width(&self, width: Option<f32>, cx: &mut App) {
        self.entity.update(cx, |pane, cx| {
            let width = width.filter(|width| width.is_finite() && *width > 0.0);
            pane.options.content_width = width;
            if let Some(renderer) = &pane.renderer {
                renderer.update(cx, |pane, cx| {
                    pane.hosted_content_width = width.map(px);
                    cx.notify();
                });
            }
            cx.notify();
        });
    }
    fn restore_scroll_anchor(&self, anchor: DiffScrollAnchor, cx: &mut App) {
        self.entity.update(cx, |pane, cx| {
            pane.reveal_at(anchor.side, anchor.line, true, cx)
        });
    }
    fn set_file_navigation(
        &self,
        navigation: gitcomet_extension_api::DiffFileNavigation,
        cx: &mut App,
    ) {
        self.entity.update(cx, |pane, cx| {
            pane.options.file_navigation = navigation;
            pane.sync_decor(cx);
            cx.notify();
        });
    }
    fn set_bar(&self, bar: gitcomet_extension_api::DiffBar, cx: &mut App) {
        self.entity.update(cx, |pane, cx| {
            pane.options.bar = bar;
            pane.sync_decor(cx);
            cx.notify();
        });
    }
    fn scroll_anchor(&self, cx: &App) -> Option<DiffScrollAnchor> {
        let pane = self.entity.read(cx);
        if let Some(renderer) = &pane.renderer {
            return renderer.read(cx).hosted_scroll_anchor();
        }
        None
    }
}

fn read_only_policy(policy: &mut DiffPanePolicy) {
    policy.allow_edit = false;
    policy.allow_stage = false;
    policy.allow_annotate = false;
    policy.blame = false;
    policy.file_navigation = false;
}
