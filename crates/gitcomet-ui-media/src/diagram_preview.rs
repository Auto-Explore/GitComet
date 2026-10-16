//! The same asynchronous diagram surface is used by files and Markdown blocks.
use crate::rendered_preview::RenderedPreviewKind;
use crate::rendered_preview::RenderedPreviewMode;
use gitcomet_base::Loadable;
use gitcomet_diagrams::DiagramKind;
use gitcomet_diagrams::source::DiagramKey;
use gitcomet_ui_kit::theme::AppTheme;
use gpui::AnyElement;
use gpui::App;
use gpui::ElementId;
use gpui::IntoElement;
use gpui::Window;
use gpui::div;
use gpui::prelude::*;
use gpui::px;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiagramInput {
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

mod cache;
use cache::*;
mod viewport;
use viewport::*;
pub mod block;
use block::*;

/// Where a diagram preview sits, and what it shows.
#[derive(Clone)]
pub enum DiagramPlacement {
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
            let result =
                gitcomet_ui_kit::ui_runtime::background_compute(move || match input.read() {
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
                let scale =
                    gitcomet_ui_kit::ui_scale::UiScale::current(cx).with_appearance(theme.metrics);
                if let Some(error) = &self.tile_error {
                    body = body.child(status_line(error.clone(), theme, scale));
                }
                for diagnostic in &diagram.diagnostics {
                    body = body.child(status_line(diagnostic.clone(), theme, scale));
                }
            }
            Loadable::Error(error) => {
                let scale =
                    gitcomet_ui_kit::ui_scale::UiScale::current(cx).with_appearance(theme.metrics);
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
pub struct DiagramPreview {
    pub id: ElementId,
    pub theme: AppTheme,
    pub source_element: Option<AnyElement>,
    pub force_source: bool,
    pub placement: DiagramPlacement,
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
#[cfg(any(test, feature = "test-support"))]
pub fn cached_tiles_for_tests(cx: &App) -> Vec<(u8, u32)> {
    let inner = cx.global::<AppDiagramCache>().0.inner.lock().unwrap();
    inner
        .tiles
        .iter()
        .map(|(key, _)| (key.zoom, key.row))
        .collect()
}

/// Diagnostics of every prepared diagram, for tests that drive the real window.
#[cfg(any(test, feature = "test-support"))]
pub fn cached_diagnostics_for_tests(cx: &App) -> usize {
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
pub mod benchmarks;
