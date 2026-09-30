//! A hosted diff pane: an independently owned view of one diff session, or
//! of two texts handed to it. Its selection, scroll, search, encoding, and
//! rows are its own; a session pane observes only its session's revision and
//! closes the session when dropped.

use super::projection::{DisplayRow, PaneProjection, selected_text};
use super::rows::{PaneRow, PaneRowKind, rows_from_file_text, rows_from_patch, side_text};
use super::*;
use crate::kit::interaction::{self as controls, ControlInteractionExt as _};
use gitcomet_core::domain::DiffTarget;
use gitcomet_core::text_format::TextEncoding;
use gitcomet_extension_api::{
    DiffAnnotations, DiffInset, DiffLegendItem, DiffLineRange, DiffLineSide, DiffPaneImpl,
    DiffPaneOptions, DiffSnapshot, RepositoryHandle, StateSubscription, WindowHost,
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
    source: PaneSource,
    view_id: DiffViewId,
    options: DiffPaneOptions,
    rows: Arc<Vec<PaneRow>>,
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
    _state: Option<StateSubscription>,
}

impl DiffPaneView {
    fn with_source(host: WindowHost, source: PaneSource, options: DiffPaneOptions) -> Self {
        Self {
            host,
            source,
            view_id: DiffViewId::next(),
            options,
            rows: Arc::default(),
            projection: Arc::default(),
            insets: Arc::from([]),
            annotations: Arc::default(),
            markers: Arc::default(),
            legend: Vec::new(),
            #[cfg(test)]
            marker_builds: 0,
            rows_rev: None,
            loading: true,
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

    /// Clears what the pane showed, dropping any build still running for it.
    fn reset(&mut self, path: Option<&std::path::Path>) {
        self.language = path.and_then(crate::view::rows::diff_syntax_language_for_path);
        self.build = None;
        self.building_rev = None;
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
        self.place_markers();
        cx.notify();
    }

    pub(crate) fn set_insets(&mut self, insets: Vec<DiffInset>, cx: &mut gpui::Context<Self>) {
        self.insets = insets.into();
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
        self.reset(Some(&snapshot.path));
        self.build_rows(None, false, None, cx, move || {
            rows_from_file_text(&snapshot.old, &snapshot.new)
        });
        cx.notify();
    }

    fn session<'a>(&self, state: &'a AppState) -> Option<&'a DiffSession> {
        let PaneSource::Session {
            repository, target, ..
        } = &self.source
        else {
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
                session.target == *target
                    && session.target.old_file_path() == target.old_file_path()
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
        if self.rows_rev == Some(session.rev) || self.building_rev == Some(session.rev) {
            return;
        }
        let rev = session.rev;
        self.blame = match &session.blame {
            Loadable::Ready(lines) => Some(Arc::clone(lines)),
            _ => None,
        };
        let file_text = match &session.file_text {
            Loadable::Ready(Some(text)) => Some(Arc::clone(text)),
            _ => None,
        };
        let patch = match &session.diff {
            Loadable::Ready(diff) => Some(Arc::clone(diff)),
            _ => None,
        };
        let error = match (&session.diff, &session.file_text) {
            (Loadable::Error(error), _) | (_, Loadable::Error(error)) => {
                Some(SharedString::from(error.clone()))
            }
            _ => None,
        };
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
        self.build_rows(Some(rev), loading, error, cx, move || match file_text {
            Some(text) => {
                let old = side_text(&text, DiffLineSide::Old).unwrap_or_default();
                let new = side_text(&text, DiffLineSide::New).unwrap_or_default();
                rows_from_file_text(&old, &new)
            }
            None => patch.map(|diff| rows_from_patch(&diff)).unwrap_or_default(),
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
        build: impl FnOnce() -> Vec<PaneRow> + Send + 'static,
    ) {
        self.building_rev = rev;
        self.build = Some(cx.spawn(async move |this, cx| {
            let rows = cx.background_executor().spawn(async move { build() }).await;
            let _ = this.update(cx, |pane, cx| {
                pane.rows = Arc::new(rows);
                pane.rows_rev = rev;
                pane.loading = loading;
                pane.error = error;
                pane.build = None;
                pane.building_rev = None;
                pane.reproject();
                if let Some((side, line)) = pane.pending_reveal.take() {
                    pane.reveal(side, line, cx);
                }
                cx.notify();
            });
        }));
    }

    pub(crate) fn set_target(&mut self, target: DiffTarget, cx: &mut gpui::Context<Self>) {
        self.open(target);
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
        match self.projection.display_ix(side, line) {
            Some(ix) => {
                self.scroll.scroll_to_item(ix, gpui::ScrollStrategy::Center);
                cx.notify();
            }
            None => self.pending_reveal = Some((side, line)),
        }
    }

    pub(crate) fn set_search(&mut self, query: SharedString, cx: &mut gpui::Context<Self>) {
        if !self.options.policy.search {
            return;
        }
        self.search = query;
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
        let (Some(action), Some((side, line))) =
            (self.options.on_gutter_click.clone(), self.anchor_at(ix))
        else {
            return;
        };
        cx.defer(move |cx| action(side, line, cx));
    }

    #[cfg(test)]
    pub(crate) fn view_id(&self) -> u64 {
        self.view_id.0
    }

    pub(crate) fn selected_text(&self) -> Option<String> {
        Some(selected_text(&self.rows, self.selection?))
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
                .text_color(theme.colors.foreground.secondary)
                .bg(inset
                    .color
                    .unwrap_or_else(|| theme.colors.surface.panel.into_color()))
                .child(text)
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
        let gutter_action = self.options.on_gutter_click.is_some();
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
                let annotation = row
                    .line(DiffLineSide::Old)
                    .and_then(|line| self.annotations.get(DiffLineSide::Old, line))
                    .or_else(|| {
                        row.line(DiffLineSide::New)
                            .and_then(|line| self.annotations.get(DiffLineSide::New, line))
                    })
                    .cloned();
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
                    .child(
                        div()
                            .w(ui_scale.px(3.0))
                            .h_full()
                            .flex_none()
                            .when_some(annotation.as_ref(), |bar, annotation| {
                                bar.bg(annotation.color)
                            }),
                    )
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
                            el.child(
                                div()
                                    .flex_none()
                                    .max_w(ui_scale.px(160.0))
                                    .px(ui_scale.px(6.0))
                                    .truncate()
                                    .text_size(theme.ui_text(11.0))
                                    .text_color(theme.colors.foreground.secondary)
                                    .child(label),
                            )
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

impl Render for DiffPaneView {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
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
        let markers = Arc::clone(&self.markers);
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
                    .child(
                        uniform_list(
                            "hosted_diff_rows",
                            self.projection.len(),
                            cx.processor(|this, range, _window, cx| this.render_rows(range, cx)),
                        )
                        .track_scroll(&self.scroll)
                        .size_full(),
                    )
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

/// The extension-facing handle; it owns the pane.
pub(crate) struct HostedDiffPane {
    pub(crate) entity: Entity<DiffPaneView>,
}

impl DiffPaneImpl for HostedDiffPane {
    fn view(&self) -> gpui::AnyView {
        self.entity.clone().into()
    }

    fn target(&self, cx: &App) -> Option<DiffTarget> {
        match &self.entity.read(cx).source {
            PaneSource::Session { target, .. } => Some(target.clone()),
            PaneSource::Snapshot(_) => None,
        }
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
        self.entity.read(cx).selection
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
        self.entity.read(cx).selected_text()
    }
}
