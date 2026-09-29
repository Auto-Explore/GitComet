use super::*;
use crate::kit::interaction as controls;
use crate::view::components::{ControlInteractionExt, InteractionState, InteractionStyle};
use crate::view::mod_helpers::TextFormatMenuSection;

impl MainPaneView {
    /// The strip under a file view: `[encoding ▾] [line ending ▾]`. Only
    /// shown when the view knows how the file was read.
    pub(super) fn text_format_strip(&self, cx: &mut gpui::Context<Self>) -> Option<AnyElement> {
        let status = self.text_format_status()?;
        let theme = self.theme;
        let ui_scale_percent = crate::ui_scale::UiScale::current(cx).percent();
        let scaled_px = crate::ui_scale::scaler(ui_scale_percent);

        let chip = |id: &'static str,
                    label: SharedString,
                    tooltip: SharedString,
                    warning: bool,
                    section: TextFormatMenuSection,
                    cx: &mut gpui::Context<Self>| {
            let invoker: SharedString = id.into();
            let active = self
                .active_context_menu_invoker
                .as_ref()
                .is_some_and(|open| open == &invoker);
            let color = if warning {
                theme.colors.status.warning.foreground
            } else {
                theme.colors.foreground.secondary
            };
            div()
                .id(id)
                .flex()
                .items_center()
                .gap_1()
                .px_1()
                .h(scaled_px(20.0))
                .rounded(px(theme.radii.row))
                .tab_index(0)
                .control_interaction(
                    InteractionStyle::header(theme),
                    InteractionState::default().open(active),
                )
                .child(
                    div()
                        .whitespace_nowrap()
                        .text_size(theme.ui_text(12.0))
                        .text_color(color)
                        .child(if warning {
                            SharedString::from(format!("⚠ {label}"))
                        } else {
                            label
                        }),
                )
                .child(svg_icon(
                    "icons/chevron_down.svg",
                    theme.colors.foreground.secondary,
                    scaled_px(10.0),
                ))
                .gitcomet_tooltip(theme, tooltip)
                .on_activate(
                    false,
                    controls::ControlActivation::Action,
                    cx.listener(move |this, e: &ClickEvent, window, cx| {
                        this.open_popover_at(
                            PopoverKind::TextFormatMenu { section }.invoked_by(invoker.clone()),
                            e.position(),
                            window,
                            cx,
                        );
                    }),
                )
        };

        let encoding = chip(
            "text_format_encoding",
            status.encoding_label.clone(),
            status.encoding_tooltip.clone(),
            status.encoding_warning,
            TextFormatMenuSection::Encoding,
            cx,
        );
        let line_ending = status.line_ending_label.clone().map(|label| {
            chip(
                "text_format_line_ending",
                label,
                status.line_ending_tooltip.clone(),
                false,
                TextFormatMenuSection::LineEnding,
                cx,
            )
        });
        let tab = chip(
            "text_format_tab_size",
            status.tab_label.clone(),
            status.tab_tooltip.clone(),
            false,
            TextFormatMenuSection::TabSize,
            cx,
        );
        Some(
            div()
                .id("text_format_strip")
                .debug_selector(|| "text_format_strip".to_string())
                .flex()
                .items_center()
                .justify_end()
                .gap_1()
                .px_2()
                .py_0p5()
                .bg(crate::theme::content_header_bg(theme))
                .border_t_1()
                .border_color(theme.colors.stroke.default)
                .child(encoding)
                .when_some(line_ending, |d, chip| d.child(chip))
                .child(tab)
                .into_any_element(),
        )
    }
}
