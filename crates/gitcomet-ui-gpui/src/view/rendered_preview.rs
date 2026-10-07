//! Shared controls and file/comparison layout for rendered previews.
use super::*;

pub(super) fn mode_toggle<V: 'static>(
    kind: RenderedPreviewKind,
    mode: RenderedPreviewMode,
    preview_disabled: bool,
    theme: AppTheme,
    cx: &mut gpui::Context<V>,
    select: impl Fn(&mut V, RenderedPreviewMode, &mut Window, &mut gpui::Context<V>) + Clone + 'static,
) -> gpui::Stateful<gpui::Div> {
    let select_preview = select.clone();
    div()
        .id(kind.toggle_id())
        .debug_selector(move || kind.toggle_id().to_string())
        .flex()
        .items_center()
        .gap_1()
        .child(
            components::Button::new(kind.rendered_button_id(), kind.rendered_label())
                .style(if mode == RenderedPreviewMode::Rendered {
                    components::ButtonStyle::Filled
                } else {
                    components::ButtonStyle::Outlined
                })
                .disabled(preview_disabled)
                .on_click(theme, cx, move |this, _, window, cx| {
                    select_preview(this, RenderedPreviewMode::Rendered, window, cx);
                }),
        )
        .child(
            components::Button::new(kind.source_button_id(), kind.source_label())
                .style(if mode == RenderedPreviewMode::Source {
                    components::ButtonStyle::Filled
                } else {
                    components::ButtonStyle::Outlined
                })
                .on_click(theme, cx, move |this, _, window, cx| {
                    select(this, RenderedPreviewMode::Source, window, cx);
                }),
        )
}

pub(super) enum PreviewContent {
    File(AnyElement),
    Diff { old: AnyElement, new: AnyElement },
}

pub(super) fn render(
    id: &'static str,
    content: PreviewContent,
    theme: AppTheme,
    ui_scale_percent: u32,
) -> AnyElement {
    let cell_padding = crate::ui_scale::design_px_from_percent(16.0, ui_scale_percent);
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
