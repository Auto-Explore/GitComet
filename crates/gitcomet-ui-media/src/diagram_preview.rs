//! The same asynchronous diagram surface is used by files and Markdown blocks.
use crate::rendered_preview::RenderedPreviewKind;
use crate::rendered_preview::RenderedPreviewMode;
use gitcomet_base::Loadable;
use gitcomet_diagrams::DiagramKind;
use gitcomet_ui_kit::theme::AppTheme;
use gpui::AnyElement;
use gpui::App;
use gpui::ElementId;
use gpui::IntoElement;
use gpui::Window;
use gpui::div;
use gpui::prelude::*;
use gpui::px;
use std::path::PathBuf;
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
}

impl DiagramInput {
    fn read(&self) -> Result<(DiagramKind, Arc<str>), String> {
        match self {
            Self::TooLarge { .. } => Err("Diagram exceeds the 1 MiB preview limit.".into()),
            Self::Text { kind, text } => Ok((*kind, text.clone())),
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
                Ok((*kind, text.into()))
            }
        }
    }
}

mod cache;
use cache::*;
mod viewport;
use viewport::*;

struct DiagramState {
    cache: Arc<DiagramCache>,
    viewport: Viewport,
    generation: Arc<AtomicU64>,
    source_generation: Arc<AtomicU64>,
    tile_task: Option<gpui::Task<()>>,
    requested: Vec<TileKey>,
    tile_error: Option<String>,
    input: DiagramInput,
    result: Loadable<Arc<RasterDiagram>>,
    task: Option<gpui::Task<()>>,
    seq: u64,
    preview_mode: RenderedPreviewMode,
}

impl DiagramState {
    fn new(input: DiagramInput, cache: Arc<DiagramCache>, cx: &mut gpui::Context<Self>) -> Self {
        let mut state = Self {
            cache: cache.clone(),
            viewport: Viewport::default(),
            generation: Arc::new(AtomicU64::new(0)),
            source_generation: Arc::new(AtomicU64::new(0)),
            tile_task: None,
            requested: Vec::new(),
            tile_error: None,
            input,
            result: Loadable::NotLoaded,
            task: None,
            seq: 0,
            preview_mode: RenderedPreviewMode::Rendered,
        };
        state.load(cache, cx);
        state
    }

    fn load(&mut self, cache: Arc<DiagramCache>, cx: &mut gpui::Context<Self>) {
        self.viewport = Viewport::default();
        self.invalidate_tiles();
        self.cache = cache.clone();
        self.seq = self.seq.wrapping_add(1);
        let seq = self.seq;
        self.source_generation.store(seq, Ordering::Relaxed);
        let generation = self.source_generation.clone();
        self.result = Loadable::Loading;
        let input = self.input.clone();
        self.task = Some(cx.spawn(async move |view, cx| {
            let result =
                gitcomet_ui_kit::ui_runtime::background_compute(move || match input.read() {
                    Ok((kind, source)) => cache.render_current(kind, source, &generation, seq),
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
        embedded: bool,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let failed = matches!(self.result, Loadable::Error(_));
        let mode = if force_source || failed {
            RenderedPreviewMode::Source
        } else {
            self.preview_mode
        };
        let mut body = div()
            .w_full()
            .min_w(px(0.0))
            .flex()
            .flex_col()
            .gap_2()
            .when(!embedded, |d| d.h_full());
        if embedded {
            body = body.child(super::rendered_preview::mode_toggle(
                RenderedPreviewKind::Diagram,
                mode,
                failed,
                theme,
                cx,
                |this, mode, _, cx| {
                    this.preview_mode = mode;
                    cx.notify();
                },
            ));
            if mode == RenderedPreviewMode::Source && !failed {
                return body.children(fallback).into_any_element();
            }
        }
        match self.result.clone() {
            Loadable::Ready(diagram) => {
                body = body.child(self.render_viewport(diagram.clone(), theme, embedded, cx));
                if let Some(error) = &self.tile_error {
                    body = body.child(
                        div()
                            .text_color(theme.colors.status.warning.foreground)
                            .child(error.clone()),
                    );
                }
                for diagnostic in &diagram.diagnostics {
                    body = body.child(
                        div()
                            .text_color(theme.colors.status.warning.foreground)
                            .child(diagnostic.clone()),
                    );
                }
            }
            Loadable::Error(error) => {
                body = body.child(
                    div()
                        .debug_selector(|| "diagram_error".into())
                        .text_color(theme.colors.status.warning.foreground)
                        .child(error.clone()),
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
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.source_generation.fetch_add(1, Ordering::Relaxed);
    }
}

mod surface;

#[derive(gpui::IntoElement)]
pub struct DiagramPreview {
    pub id: ElementId,
    pub input: DiagramInput,
    pub theme: AppTheme,
    pub source_element: Option<AnyElement>,
    pub force_source: bool,
    pub embedded: bool,
}

impl gpui::RenderOnce for DiagramPreview {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let cache = cache(cx);
        let initial = self.input.clone();
        let state = window.use_keyed_state(self.id.clone(), cx, |_, cx| {
            DiagramState::new(initial, cache.clone(), cx)
        });
        let body = state.update(cx, |state, cx| {
            if state.input != self.input {
                state.input = self.input;
                state.load(cache, cx);
            }
            state.render(
                self.theme,
                self.source_element,
                self.force_source,
                self.embedded,
                cx,
            )
        });
        div()
            .id(self.id)
            .debug_selector(|| "diagram_preview".into())
            .w_full()
            .min_w(px(0.0))
            .when(!self.embedded, |d| d.size_full())
            .child(body)
    }
}

#[cfg(test)]
mod tests;

#[cfg(feature = "benchmarks")]
pub mod benchmarks;
