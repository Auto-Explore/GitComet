//! The same asynchronous diagram surface is used by files and Markdown blocks.
use super::*;
use gitcomet_diagrams::DiagramKind;
use std::path::PathBuf;
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
                input,
                theme,
                source_element: None,
                force_source: false,
                embedded: false,
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
            let result = crate::ui_runtime::background_compute(move || match input.read() {
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
pub(in crate::view) struct DiagramPreview {
    pub(in crate::view) id: ElementId,
    pub(in crate::view) input: DiagramInput,
    pub(in crate::view) theme: AppTheme,
    pub(in crate::view) source_element: Option<AnyElement>,
    pub(in crate::view) force_source: bool,
    pub(in crate::view) embedded: bool,
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
pub(crate) mod benchmarks;
