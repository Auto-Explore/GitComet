//! The same asynchronous diagram surface is used by files and Markdown blocks.
use super::*;
use gitcomet_diagrams::DiagramKind;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) enum DiagramInput {
    TooLarge {
        revision: Arc<str>,
    },
    Text {
        kind: DiagramKind,
        text: Arc<str>,
    },
    File {
        kind: DiagramKind,
        path: PathBuf,
        revision: Arc<str>,
    },
    /// A diagram whose source was parsed and hashed already.
    Key(DiagramKey),
}

impl DiagramInput {
    pub(in crate::view) fn from_side(
        kind: DiagramKind,
        source: Option<&gitcomet_core::domain::FileDiffTextSource>,
        text: Option<&Arc<str>>,
    ) -> Option<Self> {
        if let Some(text) = text {
            return Some(Self::Text {
                kind,
                text: text.clone(),
            });
        }
        source.map(|source| Self::File {
            kind,
            path: source.path.clone(),
            revision: source.identity.clone(),
        })
    }

    fn read(&self) -> Result<DiagramKey, String> {
        match self {
            Self::TooLarge { .. } => Err("Diagram exceeds the 1 MiB preview limit.".into()),
            Self::Text { kind, text } => Ok(DiagramKey::new(*kind, text.clone())),
            Self::Key(key) => Ok(key.clone()),
            Self::File { kind, path, .. } => {
                use std::io::Read as _;
                let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
                let mut bytes = Vec::new();
                file.take(gitcomet_diagrams::MAX_SOURCE_BYTES as u64 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|error| error.to_string())?;
                if bytes.len() > gitcomet_diagrams::MAX_SOURCE_BYTES {
                    return Err("Diagram exceeds the 1 MiB preview limit.".into());
                }
                let text = String::from_utf8(bytes).map_err(|error| error.to_string())?;
                Ok(DiagramKey::new(*kind, text.into()))
            }
        }
    }
}

impl MainPaneView {
    fn diagram_kind(&self) -> Option<DiagramKind> {
        let path = match self.rendered_diff_target()? {
            DiffTarget::WorkingTree { path, .. }
            | DiffTarget::Commit { path, .. }
            | DiffTarget::CommitRange {
                path: Some(path), ..
            } => path,
            _ => return None,
        };
        DiagramKind::for_path(path)
    }

    fn diagram_inputs(&self, kind: DiagramKind) -> (Option<DiagramInput>, Option<DiagramInput>) {
        if self.main_pane_surface().file_preview {
            let input = matches!(self.worktree_preview, Loadable::Ready(_))
                .then(|| {
                    if !self.worktree_preview_text.is_empty()
                        || self.worktree_preview_source_len == 0
                    {
                        Some(DiagramInput::Text {
                            kind,
                            text: Arc::from(self.worktree_preview_text.as_ref()),
                        })
                    } else if let Some(path) = self.worktree_preview_source_path.as_ref() {
                        Some(DiagramInput::File {
                            kind,
                            path: path.clone(),
                            revision: format!(
                                "{}:{}",
                                self.worktree_preview_content_rev, self.worktree_preview_source_len
                            )
                            .into(),
                        })
                    } else {
                        Some(DiagramInput::Text {
                            kind,
                            text: Arc::from(self.worktree_preview_text.as_ref()),
                        })
                    }
                })
                .flatten();
            return if self.diff_preview_is_new_file {
                (None, input)
            } else {
                (input, None)
            };
        }
        match self.rendered_file_diff_loadable() {
            Some(Loadable::Ready(Some(file))) => (
                DiagramInput::from_side(kind, file.old_source.as_ref(), file.old.as_ref()),
                DiagramInput::from_side(kind, file.new_source.as_ref(), file.new.as_ref()),
            ),
            _ => (None, None),
        }
    }

    pub(in crate::view) fn ensure_diagram_search_source(&mut self, _cx: &mut gpui::Context<Self>) {
        let search_target = self.diagram_kind().and_then(|kind| {
            Some((
                self.active_repo_id()?,
                self.rendered_diff_target()?.clone(),
                kind,
            ))
        });
        if self.diagram_search_target != search_target {
            self.diagram_search_target = search_target;
            if self.diff_search_active && self.diagram_search_target.is_some() {
                self.rendered_preview_modes
                    .set(RenderedPreviewKind::Diagram, RenderedPreviewMode::Source);
            }
        }
    }

    pub(in crate::view) fn render_diagram_file(
        &self,
        theme: AppTheme,
        ui_scale_percent: u32,
    ) -> AnyElement {
        let Some(kind) = self.diagram_kind() else {
            return components::empty_state(theme, "Preview", "No diagram selected.")
                .into_any_element();
        };
        let (old, new) = self.diagram_inputs(kind);
        let preview = |id: &'static str, input: Option<DiagramInput>| match input {
            Some(input) => DiagramPreview {
                id: id.into(),
                theme,
                source_element: None,
                force_source: false,
                placement: DiagramPlacement::Pane(input),
            }
            .into_any_element(),
            None => components::empty_state_message(theme, "No diagram").into_any_element(),
        };
        let content = if self.main_pane_surface().file_preview {
            super::rendered_preview::PreviewContent::File(preview("diagram_file", new.or(old)))
        } else {
            super::rendered_preview::PreviewContent::Diff {
                old: preview("diagram_file_old", old),
                new: preview("diagram_file_new", new),
            }
        };
        super::rendered_preview::render("diff_diagram", content, theme, ui_scale_percent)
    }
}

mod cache;
pub(in crate::view) use cache::DiagramKey;
use cache::*;
mod viewport;
use viewport::*;

/// The viewport a markdown block's diagram folds into when collapsed.
const COLLAPSED_HEIGHT: f32 = 320.0;

/// How much of a markdown block's diagram its viewport shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BlockHeight {
    /// The whole diagram at 100%.
    Full,
    /// At most `COLLAPSED_HEIGHT`; the rest is reached by panning.
    Collapsed,
}

/// `gap_2`, between the parts of a diagram block.
const BLOCK_GAP: f32 = 8.0;
/// Design size and line height of a diagnostic or tile error under a diagram.
const STATUS_TEXT: f32 = 12.0;
const STATUS_LINE: f32 = 18.0;

/// A control row and its gap to the viewport: the Preview | Code header above,
/// the bar below.
fn block_control_row(scale: crate::ui_scale::UiScale) -> f32 {
    f32::from(components::control_height(scale) + scale.px(BLOCK_GAP))
}

/// A warning line under a diagram, at a fixed line height so an undrawn
/// block's size can count it.
fn status_line(text: String, theme: AppTheme, scale: crate::ui_scale::UiScale) -> gpui::Div {
    div()
        .text_size(scale.ui_text(STATUS_TEXT))
        .line_height(scale.ui_text(STATUS_LINE))
        .text_color(theme.colors.status.warning.foreground)
        .child(text)
}

/// What the reader chose for one markdown diagram block.
#[derive(Clone, Copy, PartialEq)]
struct BlockChoice {
    mode: RenderedPreviewMode,
    height: BlockHeight,
}

impl Default for BlockChoice {
    fn default() -> Self {
        Self {
            mode: RenderedPreviewMode::Rendered,
            height: BlockHeight::Full,
        }
    }
}

/// The reader's choices for markdown diagram blocks, by diagram, kept for the
/// session so a block rebuilt later, or shown elsewhere, opens the same way.
#[derive(Clone, Default)]
struct BlockChoices(Rc<RefCell<FxHashMap<DiagramKey, BlockChoice>>>);

impl gpui::Global for BlockChoices {}

impl BlockChoices {
    fn current(cx: &mut App) -> Self {
        cx.default_global::<Self>().clone()
    }
    fn get(&self, key: &DiagramKey) -> BlockChoice {
        self.0.borrow().get(key).copied().unwrap_or_default()
    }
    /// A choice back at the default is forgotten, and its source with it.
    fn update(&self, key: &DiagramKey, change: impl FnOnce(&mut BlockChoice)) {
        let mut choices = self.0.borrow_mut();
        let choice = choices.entry(key.clone()).or_default();
        change(choice);
        if *choice == BlockChoice::default() {
            choices.remove(key);
        }
    }
}

/// What is known about markdown diagram blocks before they are drawn: the
/// prepared scenes and the reader's choices. The default knows nothing.
#[derive(Clone, Default)]
pub(in crate::view) struct DiagramHeights {
    cache: Arc<DiagramCache>,
    choices: BlockChoices,
}

impl DiagramHeights {
    pub(in crate::view) fn new(cx: &mut App) -> Self {
        Self {
            cache: cache(cx),
            choices: BlockChoices::current(cx),
        }
    }

    /// A block's height before it is drawn, as the reader left it: its source
    /// listing (`listing` tall), or its diagram once the scene is prepared,
    /// or its error and listing if that failed, else the collapsed viewport.
    pub(in crate::view) fn estimate_block(
        &self,
        key: &DiagramKey,
        scale: crate::ui_scale::UiScale,
        listing: f32,
    ) -> f32 {
        // The controls scale with the UI; the diagram is drawn at its own size.
        let row = block_control_row(scale);
        let status = f32::from(scale.ui_text(STATUS_LINE) + scale.px(BLOCK_GAP));
        // As the block draws: a failed diagram shows its error whatever the
        // reader chose, then the source.
        let facts = self.cache.scene_facts(key);
        if facts == Some(SceneFacts::Failed) {
            return row + status + listing;
        }
        let choice = self.choices.get(key);
        if choice.mode == RenderedPreviewMode::Source {
            return row + listing;
        }
        let Some(SceneFacts::Drawn {
            height: scene,
            diagnostics,
        }) = facts
        else {
            return row + COLLAPSED_HEIGHT;
        };
        let full = full_height(scene);
        let viewport = match choice.height {
            BlockHeight::Full => full,
            BlockHeight::Collapsed => full.min(COLLAPSED_HEIGHT),
        };
        let bar = if full > COLLAPSED_HEIGHT { row } else { 0.0 };
        row + viewport + bar + diagnostics as f32 * status
    }
}

/// Where a diagram preview sits, and what it shows.
#[derive(Clone)]
pub(in crate::view) enum DiagramPlacement {
    /// Filling a pane, for a diagram file.
    Pane(DiagramInput),
    /// A block of a markdown document, which `scroll` scrolls when the host
    /// has one.
    Block {
        key: DiagramKey,
        scroll: Option<gpui::ScrollHandle>,
    },
}

impl DiagramPlacement {
    fn input(&self) -> DiagramInput {
        match self {
            Self::Pane(input) => input.clone(),
            Self::Block { key, .. } => DiagramInput::Key(key.clone()),
        }
    }
}

struct DiagramState {
    cache: Arc<DiagramCache>,
    /// This viewport's id in the cache's count of the tiles shown.
    viewport_id: u64,
    choices: BlockChoices,
    /// Shared with the scrollbars, which read it as this frame laid it out.
    viewport: Rc<RefCell<Viewport>>,
    /// How far the viewport's top was above the visible area when last drawn.
    hidden_above: f32,
    generation: Arc<AtomicU64>,
    source_generation: Arc<AtomicU64>,
    tile_task: Option<gpui::Task<()>>,
    requested: Vec<TileKey>,
    tile_error: Option<String>,
    input: DiagramInput,
    result: Loadable<Arc<RasterDiagram>>,
    task: Option<gpui::Task<()>>,
    seq: u64,
}

impl DiagramState {
    fn new(input: DiagramInput, cache: Arc<DiagramCache>, cx: &mut gpui::Context<Self>) -> Self {
        let mut state = Self {
            viewport_id: cache.viewport_id(),
            cache: cache.clone(),
            viewport: Rc::default(),
            hidden_above: 0.0,
            generation: Arc::new(AtomicU64::new(0)),
            source_generation: Arc::new(AtomicU64::new(0)),
            tile_task: None,
            requested: Vec::new(),
            tile_error: None,
            input,
            result: Loadable::NotLoaded,
            task: None,
            seq: 0,
            choices: BlockChoices::current(cx),
        };
        state.load(cache, cx);
        state
    }

    fn load(&mut self, cache: Arc<DiagramCache>, cx: &mut gpui::Context<Self>) {
        *self.viewport.borrow_mut() = Viewport::default();
        self.invalidate_tiles();
        self.cache = cache.clone();
        self.seq = self.seq.wrapping_add(1);
        let seq = self.seq;
        self.source_generation.store(seq, Ordering::Relaxed);
        // A prepared diagram is ready in this frame, rather than after one of
        // "Rendering diagram…" at a height the document would then correct.
        // Only a Key: a Text would hash its source here, a File would read disk.
        if let DiagramInput::Key(key) = &self.input
            && let Some(result) = cache.prepared(key)
        {
            self.complete(seq, result);
            return;
        }
        let generation = self.source_generation.clone();
        self.result = Loadable::Loading;
        let input = self.input.clone();
        self.task = Some(cx.spawn(async move |view, cx| {
            let result = crate::ui_runtime::background_compute(move || match input.read() {
                Ok(key) => cache.render_current(key, &generation, seq),
                Err(error) => Err(error),
            })
            .await;
            let _ = view.update(cx, |this, cx| {
                if this.complete(seq, result) {
                    cx.notify();
                }
            });
        }));
    }

    fn complete(&mut self, seq: u64, result: RasterResult) -> bool {
        if self.seq != seq {
            return false;
        }
        self.result = match result {
            Ok(result) => Loadable::Ready(result),
            Err(error) => Loadable::Error(error),
        };
        self.task = None;
        true
    }

    fn render(
        &mut self,
        theme: AppTheme,
        fallback: Option<AnyElement>,
        force_source: bool,
        placement: &DiagramPlacement,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let block = match placement {
            DiagramPlacement::Block { key, .. } => Some(key),
            DiagramPlacement::Pane(_) => None,
        };
        let embedded = block.is_some();
        let failed = matches!(self.result, Loadable::Error(_));
        let mode = match block {
            _ if force_source || failed => RenderedPreviewMode::Source,
            Some(key) => self.choices.get(key).mode,
            None => RenderedPreviewMode::Rendered,
        };
        // No tiles until the viewport, if this frame draws one, asks for them.
        self.cache.show_tiles(self.viewport_id, 0);
        let mut body = div()
            .w_full()
            .min_w(px(0.0))
            .flex()
            .flex_col()
            .gap_2()
            .when(!embedded, |d| d.h_full());
        if let Some(key) = block.cloned() {
            body = body.child(super::rendered_preview::mode_toggle(
                RenderedPreviewKind::Diagram,
                mode,
                failed,
                theme,
                cx,
                move |this, mode, _, cx| {
                    this.choices.update(&key, |choice| choice.mode = mode);
                    cx.notify();
                },
            ));
            if mode == RenderedPreviewMode::Source && !failed {
                return body.children(fallback).into_any_element();
            }
        }
        match self.result.clone() {
            Loadable::Ready(diagram) => {
                body = body.child(match placement {
                    DiagramPlacement::Pane(_) => {
                        self.render_viewport(diagram.clone(), theme, None, cx)
                    }
                    DiagramPlacement::Block { key, scroll } => {
                        self.render_block(diagram.clone(), theme, scroll.clone(), key, cx)
                    }
                });
                let scale = crate::ui_scale::UiScale::current(cx).with_appearance(theme.metrics);
                if let Some(error) = &self.tile_error {
                    body = body.child(status_line(error.clone(), theme, scale));
                }
                for diagnostic in &diagram.diagnostics {
                    body = body.child(status_line(diagnostic.clone(), theme, scale));
                }
            }
            Loadable::Error(error) => {
                let scale = crate::ui_scale::UiScale::current(cx).with_appearance(theme.metrics);
                body = body.child(
                    status_line(error.clone(), theme, scale)
                        .debug_selector(|| "diagram_error".into()),
                );
                if !embedded {
                    body = body.child("Choose Code to view the file source.");
                }
            }
            _ => {
                body = body.child(
                    div()
                        .debug_selector(|| "diagram_loading".into())
                        .text_color(theme.colors.foreground.secondary)
                        .child("Rendering diagram…"),
                );
            }
        }
        if embedded && failed {
            body = body.children(fallback);
        }
        body.into_any_element()
    }
}

impl DiagramState {
    /// A markdown block's viewport, with a bar under a diagram taller than
    /// `COLLAPSED_HEIGHT` that folds it to that height and back.
    fn render_block(
        &mut self,
        diagram: Arc<RasterDiagram>,
        theme: AppTheme,
        scroll: Option<gpui::ScrollHandle>,
        key: &DiagramKey,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let full = full_height(diagram.scene.height());
        let block_height = self.choices.get(key).height;
        let height = match block_height {
            BlockHeight::Full => full,
            BlockHeight::Collapsed => full.min(COLLAPSED_HEIGHT),
        };
        let viewport = self.render_viewport(diagram, theme, Some(height), cx);
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap_2()
            .child(viewport)
            .when(full > COLLAPSED_HEIGHT, |d| {
                d.child(self.block_height_toggle(block_height, key.clone(), scroll, theme, cx))
            })
            .into_any_element()
    }

    fn block_height_toggle(
        &self,
        block_height: BlockHeight,
        key: DiagramKey,
        scroll: Option<gpui::ScrollHandle>,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let (label, icon, next) = match block_height {
            BlockHeight::Full => (
                "Collapse diagram",
                "icons/chevron_up.svg",
                BlockHeight::Collapsed,
            ),
            BlockHeight::Collapsed => (
                "Show full diagram",
                "icons/chevron_down.svg",
                BlockHeight::Full,
            ),
        };
        let icon_size = crate::ui_scale::UiScale::current(cx).px(12.0);
        components::Button::new("diagram_block_height_toggle", label)
            .start_slot(svg_icon(icon, theme.colors.foreground.secondary, icon_size))
            .style(components::ButtonStyle::Outlined)
            .on_click(theme, cx, move |this, _, _, cx| {
                // Folding a block whose top has scrolled away would leave the
                // reader below it; scroll back just enough to keep it in view.
                if next == BlockHeight::Collapsed
                    && let Some(scroll) = &scroll
                {
                    let offset = scroll.offset();
                    let shift = px(this.hidden_above);
                    scroll.set_offset(point(offset.x, (offset.y + shift).min(px(0.0))));
                }
                this.choices.update(&key, |choice| choice.height = next);
                // Show the whole diagram, or its top, from 100%.
                *this.viewport.borrow_mut() = Viewport::default();
                this.invalidate_tiles();
                cx.notify();
            })
            .w_full()
            .into_any_element()
    }
}

impl Drop for DiagramState {
    fn drop(&mut self) {
        self.cache.show_tiles(self.viewport_id, 0);
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.source_generation.fetch_add(1, Ordering::Relaxed);
    }
}

mod pan_scrollbar;
mod surface;

#[derive(gpui::IntoElement)]
pub(in crate::view) struct DiagramPreview {
    pub(in crate::view) id: ElementId,
    pub(in crate::view) theme: AppTheme,
    pub(in crate::view) source_element: Option<AnyElement>,
    pub(in crate::view) force_source: bool,
    pub(in crate::view) placement: DiagramPlacement,
}

impl gpui::RenderOnce for DiagramPreview {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let cache = cache(cx);
        let input = self.placement.input();
        let initial = input.clone();
        let state = window.use_keyed_state(self.id.clone(), cx, |_, cx| {
            DiagramState::new(initial, cache.clone(), cx)
        });
        let body = state.update(cx, |state, cx| {
            // Keep the newest input even when equal: a re-parse brings a new
            // source `Arc`, and later frames then compare by pointer.
            let changed = state.input != input;
            state.input = input;
            if changed {
                state.load(cache, cx);
            }
            state.render(
                self.theme,
                self.source_element,
                self.force_source,
                &self.placement,
                cx,
            )
        });
        div()
            .id(self.id)
            .debug_selector(|| "diagram_preview".into())
            .w_full()
            .min_w(px(0.0))
            .when(matches!(self.placement, DiagramPlacement::Pane(_)), |d| {
                d.size_full()
            })
            .child(body)
    }
}

/// Zoom step and row of every rasterized tile, for tests that drive the real
/// window.
#[cfg(test)]
pub(in crate::view) fn cached_tiles_for_tests(cx: &App) -> Vec<(u8, u32)> {
    let inner = cx.global::<AppDiagramCache>().0.inner.lock().unwrap();
    inner
        .tiles
        .iter()
        .map(|(key, _)| (key.zoom, key.row))
        .collect()
}

#[cfg(test)]
pub(in crate::view) use cache::take_sources_hashed_for_tests;

/// Diagnostics of every prepared diagram, for tests that drive the real window.
#[cfg(test)]
pub(in crate::view) fn cached_diagnostics_for_tests(cx: &App) -> usize {
    let inner = cx.global::<AppDiagramCache>().0.inner.lock().unwrap();
    inner
        .entries
        .iter()
        .filter_map(|(_, result)| result.as_ref().ok())
        .map(|diagram| diagram.diagnostics.len())
        .sum()
}

#[cfg(test)]
mod tests;

#[cfg(feature = "benchmarks")]
pub(crate) mod benchmarks;
