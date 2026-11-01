//! Shared controls and file/comparison layout for rendered previews.
use gitcomet_ui_kit::components;
use gitcomet_ui_kit::theme::AppTheme;
use gitcomet_ui_kit::ui_scale;
use gpui::AnyElement;
use gpui::SharedString;
use gpui::Window;
use gpui::div;
use gpui::prelude::*;
use gpui::px;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RenderedPreviewKind {
    Svg,
    Markdown,
    Diagram,
}

impl RenderedPreviewKind {
    pub fn rendered_label(self) -> &'static str {
        match self {
            Self::Svg => "Image",
            Self::Markdown | Self::Diagram => "Preview",
        }
    }

    pub fn source_label(self) -> &'static str {
        match self {
            Self::Svg | Self::Diagram => "Code",
            Self::Markdown => "Text",
        }
    }

    pub fn rendered_button_id(self) -> &'static str {
        match self {
            Self::Svg => "svg_diff_view_image",
            Self::Markdown => "markdown_diff_view_preview",
            Self::Diagram => "diagram_diff_view_preview",
        }
    }

    pub fn toggle_id(self) -> &'static str {
        match self {
            Self::Svg => "svg_diff_view_toggle",
            Self::Markdown => "markdown_diff_view_toggle",
            Self::Diagram => "diagram_diff_view_toggle",
        }
    }

    pub fn source_button_id(self) -> &'static str {
        match self {
            Self::Svg => "svg_diff_view_code",
            Self::Markdown => "markdown_diff_view_text",
            Self::Diagram => "diagram_diff_view_code",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderedPreviewMode {
    Rendered,
    Source,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenderedPreviewModes {
    pub svg: RenderedPreviewMode,
    pub markdown: RenderedPreviewMode,
    pub diagram: RenderedPreviewMode,
    /// Markdown shows Source because a file's diff was too big to render, not
    /// because the reader chose it; the next file gets Rendered back.
    markdown_budget_fallback: bool,
}

impl Default for RenderedPreviewModes {
    fn default() -> Self {
        Self {
            svg: RenderedPreviewMode::Rendered,
            markdown: RenderedPreviewMode::Rendered,
            diagram: RenderedPreviewMode::Rendered,
            markdown_budget_fallback: false,
        }
    }
}

impl RenderedPreviewModes {
    pub fn get(self, kind: RenderedPreviewKind) -> RenderedPreviewMode {
        match kind {
            RenderedPreviewKind::Svg => self.svg,
            RenderedPreviewKind::Markdown => self.markdown,
            RenderedPreviewKind::Diagram => self.diagram,
        }
    }

    pub fn set(&mut self, kind: RenderedPreviewKind, mode: RenderedPreviewMode) {
        match kind {
            RenderedPreviewKind::Svg => self.svg = mode,
            RenderedPreviewKind::Diagram => self.diagram = mode,
            RenderedPreviewKind::Markdown => {
                self.markdown = mode;
                self.markdown_budget_fallback = false;
            }
        }
    }

    /// Show this markdown file as source because its diff is too big to
    /// render, leaving the reader's own choice to come back with the next file.
    pub fn fall_back_to_markdown_source(&mut self) {
        if self.markdown == RenderedPreviewMode::Rendered {
            self.markdown = RenderedPreviewMode::Source;
            self.markdown_budget_fallback = true;
        }
    }

    /// The file that needed the fallback is gone.
    pub fn end_markdown_budget_fallback(&mut self) {
        if std::mem::take(&mut self.markdown_budget_fallback) {
            self.markdown = RenderedPreviewMode::Rendered;
        }
    }
}

pub fn mode_toggle<V: 'static>(
    kind: RenderedPreviewKind,
    mode: RenderedPreviewMode,
    preview_disabled: bool,
    theme: AppTheme,
    cx: &mut gpui::Context<V>,
    select: impl Fn(&mut V, RenderedPreviewMode, &mut Window, &mut gpui::Context<V>) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let scale = ui_scale::UiScale::current(cx).with_appearance(theme.metrics);
    components::SegmentedControl::new(kind.toggle_id())
        .segment(
            components::Segment::new(kind.rendered_button_id(), kind.rendered_label())
                .selected(mode == RenderedPreviewMode::Rendered)
                .disabled(preview_disabled)
                .tooltip("Show rendered preview", Vec::new()),
        )
        .segment(
            components::Segment::new(kind.source_button_id(), kind.source_label())
                .selected(mode == RenderedPreviewMode::Source)
                .tooltip("Show source text", Vec::new()),
        )
        .render(theme, scale, cx, move |this, index, window, cx| {
            let mode = if index == 0 {
                RenderedPreviewMode::Rendered
            } else {
                RenderedPreviewMode::Source
            };
            select(this, mode, window, cx);
        })
}

pub enum PreviewContent {
    File(AnyElement),
    Diff { old: AnyElement, new: AnyElement },
}

pub fn render(
    id: &'static str,
    content: PreviewContent,
    theme: AppTheme,
    ui_scale_percent: u32,
) -> AnyElement {
    let cell_padding = gitcomet_ui_kit::ui_scale::design_px_from_percent(16.0, ui_scale_percent);
    let cell = |side: &'static str, content: AnyElement| {
        div()
            .id(SharedString::from(format!("{id}_{side}")))
            .debug_selector(move || format!("{id}_{side}"))
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .min_h(px(0.0))
            .overflow_hidden()
            .flex()
            .items_center()
            .justify_center()
            .p(cell_padding)
            .child(content)
    };
    let container = div()
        .id(SharedString::from(format!("{id}_container")))
        .debug_selector(move || format!("{id}_container"))
        .relative()
        .w_full()
        .min_w(px(0.0))
        .h_full()
        .min_h(px(0.0))
        .flex()
        .flex_col()
        .bg(theme.colors.surface.canvas);
    match content {
        PreviewContent::File(content) => container
            .debug_selector(move || format!("{id}_single"))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .child(cell("single_cell", content)),
            ),
        PreviewContent::Diff { old, new } => container
            .child(components::split_columns_header(
                theme,
                ui_scale_percent,
                "A (before)",
                "B (after)",
            ))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .child(cell("left", old))
                    .child(div().w(px(1.0)).h_full().bg(theme.colors.stroke.default))
                    .child(cell("right", new)),
            ),
    }
    .into_any_element()
}
