//! A hosted diff pane: an independently owned view of one diff session, or
//! of two texts handed to it. Its selection, scroll, search, encoding, and
//! rows are its own; a session pane observes only its session's revision and
//! closes the session when dropped.

use super::projection::{DisplayRow, PaneProjection, selected_text};
use super::rows::{PaneRow, PaneRowKind, rows_from_file_rows, rows_from_patch};
use super::*;
use crate::kit::interaction::{self as controls, ControlInteractionExt as _};
use crate::view::panes::main::diff_cache::SharedFileDiffCache;
use gitcomet_core::domain::{DiffRowProvider as _, DiffTarget};
use gitcomet_core::text_format::TextEncoding;
use gitcomet_extension_api::{
    DiffAnnotations, DiffInset, DiffLayout, DiffLegendItem, DiffLineRange, DiffLineSide,
    DiffPaneEvent, DiffPaneOptions, DiffPanePolicy, DiffScrollAnchor, DiffSnapshot,
    RepositoryHandle, StateSubscription, WindowHost, panes::DiffPaneImpl,
};
use gitcomet_state::diff_session::{DiffSession, DiffSessionMsg, DiffViewId};
use palette::IntoColor;
use std::rc::Rc;

enum PaneSource {
    /// A diff session in the window's store.
    Session {
        store: std::sync::Weak<AppStore>,
        repository: RepositoryHandle,
        target: DiffTarget,
    },
    /// Texts handed to the pane; nothing loads.
    Snapshot(DiffSnapshot),
}

pub(crate) struct DiffPaneView {
    host: WindowHost,
    root: Option<WeakEntity<GitCometView>>,
    renderer: Option<Entity<MainPaneView>>,
    renderer_model: Option<Entity<AppUiModel>>,
    file_cache: Option<Arc<SharedFileDiffCache>>,
    source: PaneSource,
    view_id: DiffViewId,
    options: DiffPaneOptions,
    rows: Arc<Vec<PaneRow>>,
    raw: [Option<super::raw_lines::RawLines>; 2],
    renderer_subscription: Option<gpui::Subscription>,
    /// `rows` with insets; search, selection, and markers index it.
    projection: Arc<PaneProjection>,
    insets: Arc<[DiffInset]>,
    annotations: Arc<DiffAnnotations>,
    /// Placed when rows, insets, or annotations change, never while drawing.
    markers: Arc<Vec<(f32, gpui::Hsla)>>,
    legend: Vec<DiffLegendItem>,
    #[cfg(test)]
    pub(crate) marker_builds: usize,
    /// The session revision `rows` were built from.
    rows_rev: Option<u64>,
    loading: bool,
    pending_target: Option<DiffTarget>,
    snapshot_rev: u64,
    error: Option<SharedString>,
    blame: Option<Arc<Vec<gitcomet_core::services::BlameLine>>>,
    language: Option<crate::view::rows::DiffSyntaxLanguage>,
    build: Option<gpui::Task<()>>,
    /// The session revision `build` is building.
    building_rev: Option<u64>,
    scroll: UniformListScrollHandle,
    selection: Option<DiffLineRange>,
    search: SharedString,
    matches: Vec<usize>,
    pending_reveal: Option<(DiffLineSide, u32)>,
    pending_scroll_top: bool,
    _state: Option<StateSubscription>,
}

impl DiffPaneView {
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
                let rows = MainPaneView::render_projected_inline(pane, start..end, window, cx);
                std::hint::black_box(rows).len()
            })
        })
    }

    fn with_source(host: WindowHost, source: PaneSource, mut options: DiffPaneOptions) -> Self {
        if matches!(source, PaneSource::Snapshot(_)) {
            options.policy.allow_edit = false;
            options.policy.allow_stage = false;
            options.policy.allow_annotate = false;
            options.policy.blame = false;
            options.policy.file_navigation = false;
        }
        Self {
            host,
            root: None,
            renderer: None,
            renderer_model: None,
            file_cache: None,
            source,
            view_id: DiffViewId::next(),
            options,
            rows: Arc::default(),
            raw: [None, None],
            renderer_subscription: None,
            projection: Arc::default(),
            insets: Arc::from([]),
            annotations: Arc::default(),
            markers: Arc::default(),
            legend: Vec::new(),
            #[cfg(test)]
            marker_builds: 0,
            rows_rev: None,
            loading: true,
            pending_target: None,
            snapshot_rev: 0,
            error: None,
            blame: None,
            language: None,
            build: None,
            building_rev: None,
            scroll: UniformListScrollHandle::default(),
            selection: None,
            search: SharedString::default(),
            matches: Vec::new(),
            pending_reveal: None,
            pending_scroll_top: false,
            _state: None,
        }
    }

    pub(crate) fn new(
        host: WindowHost,
        store: std::sync::Weak<AppStore>,
        repository: RepositoryHandle,
        target: DiffTarget,
        options: DiffPaneOptions,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let weak = cx.weak_entity();
        let state = host
            .observe_state(move |_, cx| {
                let _ = weak.update(cx, |pane, cx| pane.sync(cx));
            })
            .ok();
        let source = PaneSource::Session {
            store,
            repository,
            target: target.clone(),
        };
        let mut pane = Self::with_source(host, source, options);
        pane._state = state;
        pane.open(target);
        pane
    }

    pub(crate) fn snapshot(
        host: WindowHost,
        snapshot: DiffSnapshot,
        options: DiffPaneOptions,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let mut pane = Self::with_source(host, PaneSource::Snapshot(snapshot.clone()), options);
        pane.set_snapshot(snapshot, cx);
        pane
    }

    pub(crate) fn attach_root(&mut self, root: WeakEntity<GitCometView>) {
        self.root = Some(root);
    }

    fn prepare_renderer(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        if self.renderer.is_some() {
            return;
        }
        let Some(root) = self.root.clone() else {
            return;
        };
        let Ok((mut preferences, tooltip, store)) = root.read_with(cx, |root, cx| {
            (
                root.ui_model.read(cx).preferences.clone(),
                root.tooltip_host.downgrade(),
                root.store.clone(),
            )
        }) else {
            return;
        };
        let mut pane_store = crate::view::pane_store::PaneStore::from(store.clone());
        let state = match &self.source {
            PaneSource::Session { repository, .. } => {
                pane_store.bind(
                    crate::view::pane_store::DiffBinding {
                        repo_id: repository.repo_id(),
                        lifetime: repository.lifetime(),
                        view: self.view_id,
                    },
                    self.options.policy,
                );
                pane_store.set_linked(pane_store.shows_linked_worktree(&store.snapshot()));
                pane_store.snapshot()
            }
            PaneSource::Snapshot(snapshot) => {
                let state = self.snapshot_state(snapshot);
                self.options.policy.allow_edit = false;
                self.options.policy.allow_stage = false;
                self.options.policy.allow_annotate = false;
                self.options.policy.blame = false;
                self.options.policy.file_navigation = false;
                pane_store.bind_snapshot(state.clone(), self.view_id, self.options.policy);
                state
            }
        };
        preferences.diff.content_mode = DiffContentMode::Full;
        let model = cx.new(|_| AppUiModel::new_with_preferences(state, preferences));
        let theme = self.host.theme(cx);
        let renderer = cx.new(|cx| {
            let mut pane = MainPaneView::new(
                store,
                model.clone(),
                super::super::panes::main::MainPaneInit {
                    theme,
                    view_mode: GitCometViewMode::Normal,
                    focused_mergetool_labels: None,
                    focused_mergetool_exit_code: None,
                    root_view: root,
                    tooltip_host: tooltip,
                },
                window,
                cx,
            );
            pane.store = pane_store;
            pane.set_hosted_decor(
                self.view_id.0,
                self.options.clone(),
                self.annotations.clone(),
                self.insets.clone(),
            );
            pane.hosted_decor.as_mut().unwrap().file_cache = self.file_cache.clone();
            pane.annotate_enabled = self.options.policy.blame;
            pane.diff_view = diff_view_mode(self.options.layout);
            pane.hosted_content_width = Some(
                self.options
                    .content_width
                    .map(px)
                    .unwrap_or(window.viewport_size().width),
            );
            pane
        });
        self.renderer_subscription = Some(cx.observe(&renderer, |this, renderer, cx| {
            let selection = renderer.read(cx).hosted_selection();
            if this.selection != selection {
                this.selection = selection;
                this.emit(DiffPaneEvent::SelectionChanged(selection), cx);
            }
            if let Some((side, line)) = this.pending_reveal {
                let found = renderer.update(cx, |pane, _| {
                    pane.hosted_reveal_at(side, line, this.pending_scroll_top)
                });
                if found {
                    this.pending_reveal = None;
                }
            }
            cx.notify();
        }));
        self.renderer = Some(renderer);
        self.renderer_model = Some(model);
    }

    fn snapshot_state(&self, snapshot: &DiffSnapshot) -> Arc<AppState> {
        let mut repo = RepoState::new_opening(
            RepoId(u64::MAX),
            gitcomet_core::domain::RepoSpec {
                workdir: std::path::PathBuf::new(),
            },
        );
        let target = snapshot
            .patch
            .as_ref()
            .map(|patch| patch.target.clone())
            .unwrap_or_else(|| DiffTarget::working_tree(snapshot.path.clone(), DiffArea::Unstaged));
        repo.open = Loadable::Ready(());
        repo.diff_state.diff_target = Some(target.clone());
        repo.diff_state.diff_target_rev = self.snapshot_rev;
        repo.diff_state.diff_rev = self.snapshot_rev;
        repo.diff_state.diff_file_rev = self.snapshot_rev;
        repo.diff_state.diff = Loadable::Ready(snapshot.patch.clone().unwrap_or_else(|| {
            Arc::new(gitcomet_core::domain::Diff {
                target,
                lines: Vec::new(),
            })
        }));
        repo.diff_state.diff_file =
            Loadable::Ready(self.file_cache.as_ref().map(|cache| cache.file.clone()));
        Arc::new(AppState {
            active_repo: Some(repo.id),
            repos: vec![repo],
            ..Default::default()
        })
    }

    fn sync_renderer(&mut self, state: Arc<AppState>, cx: &mut gpui::Context<Self>) {
        if let (Some(renderer), Some(model)) = (&self.renderer, &self.renderer_model) {
            let linked = renderer.read(cx).store.shows_linked_worktree(&state);
            renderer.update(cx, |pane, _| pane.store.set_linked(linked));
            let state = renderer.read(cx).store.project(state);
            model.update(cx, |model, cx| model.set_state(state, cx));
        }
    }

    fn set_file_cache(
        &mut self,
        cache: Option<Arc<SharedFileDiffCache>>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.file_cache = cache;
        if let Some(renderer) = &self.renderer {
            renderer.update(cx, |pane, _| {
                pane.hosted_decor.as_mut().unwrap().file_cache = self.file_cache.clone();
            });
        }
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

    /// Clears what the pane showed, dropping any build still running for it.
    fn reset(&mut self, path: Option<&std::path::Path>) {
        self.language = path.and_then(crate::view::rows::diff_syntax_language_for_path);
        self.build = None;
        self.building_rev = None;
        self.file_cache = None;
        self.rows = Arc::default();
        self.loading = true;
        self.error = None;
        self.blame = None;
        self.selection = None;
        self.pending_reveal = None;
        self.reproject();
    }

    /// Rebuilds display rows, search matches, and markers from `rows` and
    /// `insets`.
    fn reproject(&mut self) {
        self.projection = Arc::new(PaneProjection::build(&self.rows, &self.insets));
        self.refresh_search();
        self.place_markers();
    }

    fn place_markers(&mut self) {
        self.markers = Arc::new(self.projection.markers(&self.annotations));
        #[cfg(test)]
        {
            self.marker_builds += 1;
        }
    }

    pub(crate) fn set_annotations(
        &mut self,
        annotations: Arc<DiffAnnotations>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.annotations = annotations;
        self.sync_decor(cx);
        self.place_markers();
        cx.notify();
    }

    pub(crate) fn set_insets(&mut self, insets: Vec<DiffInset>, cx: &mut gpui::Context<Self>) {
        self.insets = insets.into();
        self.sync_decor(cx);
        self.reproject();
        cx.notify();
    }

    pub(crate) fn set_legend(&mut self, legend: Vec<DiffLegendItem>, cx: &mut gpui::Context<Self>) {
        self.legend = legend;
        cx.notify();
    }

    fn open(&mut self, target: DiffTarget) {
        if !matches!(self.source, PaneSource::Session { .. }) {
            return;
        }
        self.reset(target.file_path());
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

    pub(crate) fn set_snapshot(&mut self, snapshot: DiffSnapshot, cx: &mut gpui::Context<Self>) {
        let PaneSource::Snapshot(shown) = &mut self.source else {
            return;
        };
        *shown = snapshot.clone();
        self.snapshot_rev = self.snapshot_rev.wrapping_add(1);
        self.reset(Some(&snapshot.path));
        let cache = snapshot.patch.is_none().then(|| {
            Arc::new(SharedFileDiffCache::new(
                Arc::new(gitcomet_core::domain::FileDiffText::new_shared(
                    snapshot.path.clone(),
                    Some(snapshot.old.clone()),
                    Some(snapshot.new.clone()),
                )),
                None,
                std::path::PathBuf::new(),
            ))
        });
        self.set_file_cache(cache.clone(), cx);
        if let Some(model) = &self.renderer_model {
            let state = self.snapshot_state(&snapshot);
            if let Some(renderer) = &self.renderer {
                renderer.read(cx).store.replace_snapshot(state.clone());
            }
            model.update(cx, |model, cx| model.set_state(state, cx));
        }
        self.build_rows(None, false, None, cx, move || {
            if let Some(patch) = snapshot.patch {
                return Ok((rows_from_patch(&patch), [None, None]));
            }
            let raw = [
                Some(super::raw_lines::RawLines::new(
                    Arc::from(snapshot.old.as_bytes()),
                    TextEncoding::UTF_8,
                )),
                Some(super::raw_lines::RawLines::new(
                    Arc::from(snapshot.new.as_bytes()),
                    TextEncoding::UTF_8,
                )),
            ];
            let cache = cache.expect("text snapshots have a prepared file");
            let prepared = cache.build().map_err(ToString::to_string)?;
            Ok((
                rows_from_file_rows(
                    prepared
                        .row_provider
                        .slice(0, prepared.row_provider.len_hint()),
                ),
                raw,
            ))
        });
        cx.notify();
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

    /// Rebuilds rows when this pane's session moved; other sessions and
    /// History never wake it.
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
        if self.rows_rev == Some(session.rev) || self.building_rev == Some(session.rev) {
            return;
        }
        if session.diff_target.is_none() {
            self.reset(None);
            self.set_file_cache(None, cx);
            self.sync_renderer(Arc::clone(&state), cx);
            self.loading = false;
            self.rows_rev = Some(session.rev);
            self.emit(DiffPaneEvent::TargetChanged(None), cx);
            cx.notify();
            return;
        }
        let rev = session.rev;
        self.blame = match &session.blame {
            Loadable::Ready(lines) => Some(Arc::clone(lines)),
            _ => None,
        };
        let file_text = match &session.diff_file {
            Loadable::Ready(Some(text)) => Some(Arc::clone(text)),
            _ => None,
        };
        let patch = match &session.diff {
            Loadable::Ready(diff) => Some(Arc::clone(diff)),
            _ => None,
        };
        let cache = file_text.as_ref().map(|text| {
            let workdir = match &self.source {
                PaneSource::Session { repository, .. } => repository.workdir().to_path_buf(),
                PaneSource::Snapshot(_) => unreachable!(),
            };
            self.file_cache
                .as_ref()
                .filter(|cache| cache.matches(text, patch.as_ref(), &workdir))
                .cloned()
                .unwrap_or_else(|| {
                    Arc::new(SharedFileDiffCache::new(
                        text.clone(),
                        patch.as_ref(),
                        workdir,
                    ))
                })
        });
        self.set_file_cache(cache.clone(), cx);
        self.sync_renderer(Arc::clone(&state), cx);
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
        let loading = session.is_loading();
        if file_text.is_none() && patch.is_none() {
            // A reload can overtake a row build. Do not let that build put
            // the previous generation back on screen after this notification.
            self.build = None;
            self.building_rev = None;
            self.rows_rev = Some(rev);
            self.loading = loading;
            self.error = error;
            cx.notify();
            return;
        }
        self.build_rows(Some(rev), loading, error, cx, move || match cache {
            Some(cache) => {
                let prepared = cache.build().map_err(ToString::to_string)?;
                Ok((
                    rows_from_file_rows(
                        prepared
                            .row_provider
                            .slice(0, prepared.row_provider.len_hint()),
                    ),
                    super::raw_lines::RawLines::file(&cache.file),
                ))
            }
            None => Ok((
                patch.map(|diff| rows_from_patch(&diff)).unwrap_or_default(),
                [None, None],
            )),
        });
    }

    /// Builds rows off the UI thread (reading file-backed sides and planning
    /// rows can take a while); a newer build replaces this one.
    fn build_rows(
        &mut self,
        rev: Option<u64>,
        loading: bool,
        error: Option<SharedString>,
        cx: &mut gpui::Context<Self>,
        build: impl FnOnce() -> Result<(Vec<PaneRow>, [Option<super::raw_lines::RawLines>; 2]), String>
        + Send
        + 'static,
    ) {
        self.building_rev = rev;
        self.build = Some(cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { build() }).await;
            let _ = this.update(cx, |pane, cx| {
                let (rows, raw) = match result {
                    Ok(result) => result,
                    Err(error) => {
                        let error = SharedString::from(error);
                        pane.error = Some(error.clone());
                        pane.loading = false;
                        pane.rows_rev = rev;
                        pane.build = None;
                        pane.building_rev = None;
                        pane.emit(DiffPaneEvent::Error(error), cx);
                        cx.notify();
                        return;
                    }
                };
                pane.rows = Arc::new(rows);
                pane.raw = raw;
                if !loading {
                    pane.emit(DiffPaneEvent::Loaded, cx);
                }
                pane.rows_rev = rev;
                pane.loading = loading;
                pane.error = error;
                pane.build = None;
                pane.building_rev = None;
                pane.reproject();
                if let Some((side, line)) = pane.pending_reveal.take() {
                    pane.reveal_at(side, line, pane.pending_scroll_top, cx);
                }
                cx.notify();
            });
        }));
    }

    pub(crate) fn set_target(&mut self, target: DiffTarget, cx: &mut gpui::Context<Self>) {
        self.open(target.clone());
        if let (Some(renderer), Some(model)) = (&self.renderer, &self.renderer_model) {
            let mut state = (*renderer.read(cx).store.snapshot()).clone();
            for repo in &mut state.repos {
                repo.diff_state = Default::default();
            }
            model.update(cx, |model, cx| model.set_state(Arc::new(state), cx));
        }
        self.emit(DiffPaneEvent::TargetChanged(Some(target)), cx);
        cx.notify();
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
    fn reveal_at(
        &mut self,
        side: DiffLineSide,
        line: u32,
        top: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        self.pending_scroll_top = top;
        if let Some(renderer) = &self.renderer {
            let found = renderer.update(cx, |pane, cx| {
                let found = pane.hosted_reveal_at(side, line, top);
                cx.notify();
                found
            });
            if !found {
                self.pending_reveal = Some((side, line));
            }
            return;
        }
        match self.projection.display_ix(side, line) {
            Some(ix) => {
                self.scroll.scroll_to_item(
                    ix,
                    if top {
                        gpui::ScrollStrategy::Top
                    } else {
                        gpui::ScrollStrategy::Center
                    },
                );
                cx.notify();
            }
            None => self.pending_reveal = Some((side, line)),
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
        self.refresh_search();
        cx.notify();
    }

    fn refresh_search(&mut self) {
        let query = self.search.to_lowercase();
        self.matches = if query.is_empty() {
            Vec::new()
        } else {
            (0..self.projection.len())
                .filter(|&ix| {
                    self.projection
                        .document_row(ix)
                        .and_then(|row| self.rows.get(row))
                        .is_some_and(|row| row.text.to_lowercase().contains(&query))
                })
                .collect()
        };
    }

    /// The file line at display row `ix`; `None` on insets and headers.
    fn anchor_at(&self, ix: usize) -> Option<(DiffLineSide, u32)> {
        self.rows.get(self.projection.document_row(ix)?)?.anchor()
    }

    /// A click selects the row's line; shift extends along the same side.
    fn click_row(&mut self, ix: usize, extend: bool, cx: &mut gpui::Context<Self>) {
        // Enforced here as well as by what the rows offer.
        if !self.options.policy.select_lines {
            return;
        }
        let Some((side, line)) = self.anchor_at(ix) else {
            return;
        };
        self.selection = match self.selection {
            Some(range) if extend && range.side == side => Some(DiffLineRange {
                side,
                start: range.start.min(line),
                end: range.end.max(line),
            }),
            _ => Some(DiffLineRange {
                side,
                start: line,
                end: line,
            }),
        };
        cx.notify();
    }

    /// Runs the gutter action for display row `ix`'s line, after this
    /// update so the action may read the pane.
    fn click_gutter(&mut self, ix: usize, cx: &mut gpui::Context<Self>) {
        // Enforced here as well as by what the rows offer.
        if !self.options.policy.line_action {
            return;
        }
        let (Some(action), Some((side, line))) =
            (self.options.on_gutter_click.clone(), self.anchor_at(ix))
        else {
            return;
        };
        cx.defer(move |cx| action(side, line, cx));
    }

    /// Runs the annotation action for the annotated `side`/`line`.
    fn click_annotation(&mut self, side: DiffLineSide, line: u32, cx: &mut gpui::Context<Self>) {
        if !self.options.policy.line_action || self.annotations.get(side, line).is_none() {
            return;
        }
        if let Some(action) = self.options.on_annotation_click.clone() {
            cx.defer(move |cx| action(side, line, cx));
        }
    }

    /// The display row of inset `inset`'s `line` (fallback rows).
    #[cfg(test)]
    pub(crate) fn display_row_of_inset(&self, inset: usize, line: usize) -> Option<usize> {
        let row = DisplayRow::Inset { inset, line };
        self.projection
            .display
            .iter()
            .position(|shown| *shown == row)
    }

    /// The display row showing `side`'s `line` (fallback rows).
    #[cfg(test)]
    pub(crate) fn display_row_of_line(&self, side: DiffLineSide, line: u32) -> Option<usize> {
        (0..self.projection.display.len()).find(|&ix| {
            self.projection
                .document_row(ix)
                .and_then(|row| self.rows.get(row)?.line(side))
                == Some(line)
        })
    }

    #[cfg(test)]
    pub(crate) fn view_id(&self) -> u64 {
        self.view_id.0
    }

    /// Whether the renderer offers line staging and editing for its file.
    #[cfg(test)]
    pub(crate) fn renderer_offers(&self, cx: &App) -> Option<(bool, bool)> {
        let renderer = self.renderer.as_ref()?.read(cx);
        Some((
            renderer.diff_stage_gutter_area().is_some(),
            renderer.editable_path_for_current_target().is_some(),
        ))
    }

    /// The checkout the renderer resolves the file's paths against.
    #[cfg(test)]
    pub(crate) fn renderer_workdir(&self, cx: &App) -> Option<std::path::PathBuf> {
        let renderer = self.renderer.as_ref()?.read(cx);
        renderer
            .rendered_diff_workdir()
            .map(std::path::Path::to_path_buf)
    }

    /// Sends `msg` the way the renderer's own controls would.
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

    fn render_inset_row(
        &self,
        ix: usize,
        inset: usize,
        line: usize,
        theme: AppTheme,
        ui_scale: ui_scale::UiScale,
    ) -> Option<AnyElement> {
        let inset = self.insets.get(inset)?;
        let text = inset.lines.get(line)?.clone();
        let action = inset.on_click.clone();
        let pane_id = self.view_id.0;
        Some(
            div()
                .id(("hosted_diff_row", ix))
                .debug_selector(move || format!("hosted_diff_{pane_id}_row_{ix}"))
                .h(ui_scale.px(20.0))
                .w_full()
                .flex()
                .items_center()
                .pl(ui_scale.px(if self.options.policy.line_numbers {
                    102.0
                } else {
                    14.0
                }))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_size(theme.ui_text(12.0))
                .text_color(
                    inset
                        .color
                        .unwrap_or_else(|| theme.colors.foreground.secondary.into_color()),
                )
                .bg(theme.colors.surface.panel)
                .child(text)
                .when_some(action, |row, action| {
                    row.cursor_pointer().on_activate(
                        false,
                        controls::ControlActivation::Action,
                        move |_, _, cx| {
                            cx.stop_propagation();
                            action.invoke(cx);
                        },
                    )
                })
                .into_any_element(),
        )
    }

    fn render_rows(
        &mut self,
        range: std::ops::Range<usize>,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = self.host.theme(cx);
        let ui_scale = ui_scale::UiScale::current(cx);
        let style = self.options.style;
        let policy = self.options.policy;
        let pane_id = self.view_id.0;
        let number_width = ui_scale.px(44.0);
        let gutter_action = self.options.on_gutter_click.is_some() && policy.line_action;
        let annotation_action = self.options.on_annotation_click.is_some() && policy.line_action;
        range
            .filter_map(|ix| {
                let row_ix = match *self.projection.display.get(ix)? {
                    DisplayRow::Document(row_ix) => row_ix,
                    DisplayRow::Inset { inset, line } => {
                        return self.render_inset_row(ix, inset, line, theme, ui_scale);
                    }
                };
                let row = self.rows.get(row_ix)?.clone();
                let anchor = row.anchor();
                let annotated =
                    [DiffLineSide::Old, DiffLineSide::New]
                        .into_iter()
                        .find_map(|side| {
                            let line = row.line(side)?;
                            Some((side, line, self.annotations.get(side, line)?.clone()))
                        });
                let annotation_at = annotated
                    .as_ref()
                    .filter(|_| annotation_action)
                    .map(|(side, line, _)| (*side, *line));
                let annotation = annotated.map(|(_, _, annotation)| annotation);
                let decor =
                    anchor.and_then(|(side, line)| self.options.decor.as_ref()?(side, line));
                let selected = anchor.is_some_and(|(side, line)| {
                    self.selection
                        .is_some_and(|range| range.contains(side, line))
                });
                let matched = self.matches.binary_search(&ix).is_ok();
                let background: Option<gpui::Hsla> = match row.kind {
                    PaneRowKind::Added => Some(
                        style
                            .added_background
                            .unwrap_or_else(|| theme.colors.diff.added.background.into_color()),
                    ),
                    PaneRowKind::Removed => Some(
                        style
                            .removed_background
                            .unwrap_or_else(|| theme.colors.diff.removed.background.into_color()),
                    ),
                    PaneRowKind::Context => style.context_background,
                    PaneRowKind::Hunk | PaneRowKind::Header => {
                        Some(theme.colors.surface.chrome.into_color())
                    }
                };
                let marker = match row.kind {
                    PaneRowKind::Added => "+",
                    PaneRowKind::Removed => "-",
                    _ => " ",
                };
                let blame = policy
                    .blame
                    .then(|| {
                        let line = row.line(DiffLineSide::New)? as usize;
                        let blame = self.blame.as_ref()?.get(line.checked_sub(1)?)?;
                        Some(SharedString::from(blame.author.to_string()))
                    })
                    .flatten();
                let text = row.text.clone();
                let highlights = self
                    .language
                    .filter(|_| {
                        matches!(
                            row.kind,
                            PaneRowKind::Context | PaneRowKind::Added | PaneRowKind::Removed
                        )
                    })
                    .map(|language| {
                        crate::view::rows::syntax_highlights_for_line(
                            theme,
                            &text,
                            language,
                            crate::view::rows::DiffSyntaxMode::HeuristicOnly,
                        )
                    })
                    .unwrap_or_default();
                let number = |line: Option<u32>| {
                    div()
                        .w(number_width)
                        .flex_none()
                        .text_color(theme.colors.foreground.secondary)
                        .child(line.map(|line| line.to_string()).unwrap_or_default())
                };
                let row_div = div()
                    .id(("hosted_diff_row", ix))
                    .debug_selector(move || format!("hosted_diff_{pane_id}_row_{ix}"))
                    .h(ui_scale.px(20.0))
                    .w_full()
                    .flex()
                    .items_center()
                    .overflow_hidden()
                    .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                    .text_size(theme.ui_text(13.0))
                    .text_color(theme.colors.foreground.primary)
                    .when_some(background, |row, color| row.bg(color))
                    .when_some(decor.as_ref().and_then(|decor| decor.tint), |row, tint| {
                        row.bg(tint)
                    })
                    .when(matched, |row| {
                        row.bg(theme.colors.interaction.hover_background)
                    })
                    .when(selected, |row| {
                        row.bg(theme.colors.interaction.selected_background)
                    })
                    .child(annotation_click(
                        annotation_at,
                        div()
                            .id(("hosted_diff_annotation", ix))
                            .debug_selector(move || {
                                format!("hosted_diff_{pane_id}_annotation_{ix}")
                            })
                            .w(ui_scale.px(3.0))
                            .h_full()
                            .flex_none()
                            .when_some(annotation.as_ref(), |bar, annotation| {
                                bar.bg(annotation.color)
                            }),
                        cx,
                    ))
                    .when(policy.line_numbers, |el| {
                        el.child(number(row.old_line)).child(number(row.new_line))
                    })
                    .when_some(blame, |row, author| {
                        row.child(
                            div()
                                .w(ui_scale.px(96.0))
                                .flex_none()
                                .truncate()
                                .text_color(theme.colors.foreground.secondary)
                                .child(author),
                        )
                    })
                    .child(
                        div()
                            .id(("hosted_diff_gutter", ix))
                            .debug_selector(move || format!("hosted_diff_{pane_id}_gutter_{ix}"))
                            .w(ui_scale.px(14.0))
                            .flex_none()
                            .child(
                                decor
                                    .as_ref()
                                    .and_then(|decor| decor.gutter.clone())
                                    .unwrap_or_else(|| marker.into()),
                            )
                            .when(gutter_action && anchor.is_some(), |gutter| {
                                gutter
                                    .control_interaction(
                                        controls::InteractionStyle::new(theme),
                                        controls::InteractionState::default(),
                                    )
                                    .on_activate(
                                        false,
                                        controls::ControlActivation::Nested,
                                        cx.listener(move |this, _: &gpui::ClickEvent, _, cx| {
                                            this.click_gutter(ix, cx);
                                        }),
                                    )
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(gpui::StyledText::new(text).with_highlights(highlights)),
                    )
                    .when_some(
                        annotation.and_then(|annotation| annotation.label),
                        |el, label| {
                            el.child(annotation_click(
                                annotation_at,
                                div()
                                    .id(("hosted_diff_annotation_label", ix))
                                    .debug_selector(move || {
                                        format!("hosted_diff_{pane_id}_annotation_label_{ix}")
                                    })
                                    .flex_none()
                                    .max_w(ui_scale.px(160.0))
                                    .px(ui_scale.px(6.0))
                                    .truncate()
                                    .text_size(theme.ui_text(11.0))
                                    .text_color(theme.colors.foreground.secondary)
                                    .child(label),
                                cx,
                            ))
                        },
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                            this.click_row(ix, event.modifiers.shift, cx);
                        }),
                    );
                Some(row_div.into_any_element())
            })
            .collect()
    }
}

/// An annotation's bar or label, running the annotation action for `at`.
fn annotation_click(
    at: Option<(DiffLineSide, u32)>,
    el: gpui::Stateful<gpui::Div>,
    cx: &mut gpui::Context<DiffPaneView>,
) -> gpui::Stateful<gpui::Div> {
    let Some((side, line)) = at else {
        return el;
    };
    el.cursor_pointer().on_activate(
        false,
        controls::ControlActivation::Nested,
        cx.listener(move |this, _: &gpui::ClickEvent, _, cx| {
            this.click_annotation(side, line, cx);
        }),
    )
}

impl Render for DiffPaneView {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        self.prepare_renderer(window, cx);
        let theme = self.host.theme(cx);
        let ui_scale = ui_scale::UiScale::current(cx);
        let pane_id = self.view_id.0;
        let status = if let Some(error) = &self.error {
            Some(error.clone())
        } else if self.rows.is_empty() {
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
            .unwrap_or_else(|| self.markers.clone());
        div()
            .id(("hosted_diff_pane", self.view_id.0))
            .debug_selector(move || format!("hosted_diff_{pane_id}"))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.colors.surface.canvas)
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
                        uniform_list(
                            "hosted_diff_rows",
                            self.projection.len(),
                            cx.processor(|this, range, _window, cx| this.render_rows(range, cx)),
                        )
                        .track_scroll(&self.scroll)
                        .size_full()
                        .into_any_element()
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

fn diff_view_mode(layout: DiffLayout) -> DiffViewMode {
    match layout {
        DiffLayout::Split => DiffViewMode::Split,
        _ => DiffViewMode::Inline,
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
        pane.loading || pane.build.is_some()
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
        self.entity.read(cx).matches.len()
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
        self.selection(cx)
            .map(|range| selected_text(&pane.rows, range))
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
            if let Some(renderer) = &pane.renderer {
                renderer.update(cx, |pane, cx| {
                    pane.set_diff_view_mode(diff_view_mode(layout), cx)
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
    fn scroll_anchor(&self, cx: &App) -> Option<DiffScrollAnchor> {
        let pane = self.entity.read(cx);
        if let Some(renderer) = &pane.renderer {
            return renderer.read(cx).hosted_scroll_anchor();
        }
        let (side, line) =
            pane.anchor_at(pane.scroll.0.borrow().base_handle.logical_scroll_top().0)?;
        Some(DiffScrollAnchor { side, line })
    }
}
