//! The diff pane's image and diagram previews: inputs, menus and saved
//! sampling preferences around the media crate's widgets.
use super::*;
use crate::view::diagram_preview::{DiagramInput, DiagramPlacement, DiagramPreview};
use crate::view::image_preview::{
    self, Group, ImageMenu, ImageSamplingMode, ImageSmoothFilter, Input, Preferences,
};
use gitcomet_diagrams::DiagramKind;
use std::sync::atomic::{AtomicU64, Ordering};

/// The text-format popover an image chip opens.
pub(in crate::view) fn image_menu_request(
    menu: ImageMenu,
    invoker: SharedString,
) -> PopoverRequest {
    let section = match menu {
        ImageMenu::Sampling => TextFormatMenuSection::ImageSampling,
        ImageMenu::Zoom => TextFormatMenuSection::ImageZoom,
    };
    PopoverKind::TextFormatMenu { section }.invoked_by(invoker)
}

// The generation check is inside the write lock: rapid picks cannot finish
// saving out of order, including when they come from different windows.
static SAVE_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static SAVE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

impl DiffRenderer {
    pub(in crate::view) fn set_image_preferences(
        &mut self,
        mode: Option<ImageSamplingMode>,
        filter: Option<ImageSmoothFilter>,
        cx: &mut gpui::Context<Self>,
    ) {
        let mut prefs = Preferences::current(cx);
        if let Some(mode) = mode {
            prefs.mode = mode;
        }
        if let Some(filter) = filter {
            prefs.filter = filter;
        }
        if prefs == Preferences::current(cx) {
            return;
        }
        cx.set_global(prefs);
        let sequence = SAVE_SEQUENCE.fetch_add(1, Ordering::SeqCst) + 1;
        cx.spawn(async move |view, cx| {
            let result = crate::ui_runtime::background_compute(move || {
                let _guard = SAVE_LOCK.lock().unwrap();
                if SAVE_SEQUENCE.load(Ordering::SeqCst) != sequence {
                    return Ok(());
                }
                session::persist_ui_settings(session::UiSettings {
                    image_sampling_mode: Some(prefs.mode.key().into()),
                    image_smooth_filter: Some(prefs.filter.key().into()),
                    ..Default::default()
                })
            })
            .await;
            if let Err(error) = result {
                let _ = view.update(cx, |this, cx| {
                    let report = ErrorReport::message(
                        this.active_repo_id(),
                        format!("Could not save image sampling preferences: {error}"),
                    );
                    this.commands.defer(
                        crate::view::panes::diff::services::DiffCommand::Error(report),
                        cx,
                    );
                });
            }
        })
        .detach();
        cx.notify();
    }
}

impl DiffRenderer {
    fn image_target(&self) -> String {
        format!(
            "{:?}:{:?}",
            self.active_repo_id(),
            self.rendered_diff_target()
        )
    }

    pub(in crate::view) fn begin_image_preview_frame(&mut self, cx: &mut gpui::Context<Self>) {
        self.image_preview_active = false;
        let target = self.image_target();
        self.image_preview_group.update(cx, |group, cx| {
            if group.target != target {
                group.set_inputs(target, Default::default(), false, cx);
            }
        });
    }

    pub(in crate::view) fn prepare_image_preview(
        &mut self,
        inputs: [Input; 3],
        cap_fit: bool,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<Group> {
        let group = self.image_preview_group.clone();
        self.image_preview_active = inputs.iter().any(|input| input.source.is_some());
        let target = self.image_target();
        group.update(cx, |group, cx| {
            group.set_inputs(target, inputs, cap_fit, cx)
        });
        group
    }

    pub(in crate::view) fn prepare_conflict_image_preview(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<Group> {
        let preview = &self.conflict_resolver.image_preview;
        let inputs = std::array::from_fn(|index| {
            let side = ThreeWayColumn::ALL[index];
            Input {
                source: preview.sources[index].clone(),
                preview: match preview.image(side) {
                    Loadable::Ready(Some(ConflictPreviewImage::Rendered(image))) => {
                        Some(image.clone())
                    }
                    Loadable::Ready(Some(ConflictPreviewImage::Encoded(image))) => {
                        image.clone().use_render_image(window, cx)
                    }
                    _ => None,
                },
                frame: 0,
            }
        });
        let cap = self
            .conflict_resolver
            .path
            .as_deref()
            .is_some_and(preview_path_uses_scale_down);
        self.prepare_image_preview(inputs, cap, cx)
    }

    #[cfg(test)]
    pub(in crate::view) fn image_zoom(&self, zoom: image_preview::Zoom, cx: &mut App) {
        self.image_preview_group
            .update(cx, |group, cx| group.zoom(zoom, None, cx));
    }

    pub(in crate::view) fn image_format_chips(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> Option<AnyElement> {
        if !self.image_preview_active {
            return None;
        }
        Some(image_preview::toolbar(
            self.image_preview_group.clone(),
            self.theme,
            cx,
            |this, menu, invoker, position, window, cx| {
                this.open_popover_at(image_menu_request(menu, invoker), position, window, cx);
            },
        ))
    }

    pub(in crate::view) fn image_preview_viewport(
        &self,
        group: Entity<Group>,
        side: usize,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        image_preview::viewport(
            group,
            side,
            self.theme,
            cx,
            |this, menu, invoker, position, window, cx| {
                this.open_popover_at(image_menu_request(menu, invoker), position, window, cx);
            },
        )
    }
}

fn diagram_input_from_side(
    kind: DiagramKind,
    source: Option<&gitcomet_core::domain::FileDiffTextSource>,
    text: Option<&Arc<str>>,
) -> Option<DiagramInput> {
    if let Some(text) = text {
        return Some(DiagramInput::Text {
            kind,
            text: text.clone(),
        });
    }
    source.map(|source| DiagramInput::File {
        kind,
        path: source.path.clone(),
        revision: source.identity.clone(),
    })
}

impl DiffRenderer {
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
        if self.diff_surface().file_preview {
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
                diagram_input_from_side(kind, file.old_source.as_ref(), file.old.as_ref()),
                diagram_input_from_side(kind, file.new_source.as_ref(), file.new.as_ref()),
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
        let content = if self.diff_surface().file_preview {
            rendered_preview::PreviewContent::File(preview("diagram_file", new.or(old)))
        } else {
            rendered_preview::PreviewContent::Diff {
                old: preview("diagram_file_old", old),
                new: preview("diagram_file_new", new),
            }
        };
        rendered_preview::render("diff_diagram", content, theme, ui_scale_percent)
    }
}

#[cfg(test)]
mod tests;
