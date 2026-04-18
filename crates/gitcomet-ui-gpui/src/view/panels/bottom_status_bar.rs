use super::*;

pub(in super::super) struct BottomStatusBarView {
    theme: AppTheme,
    root_view: WeakEntity<GitCometView>,
    tooltip_host: WeakEntity<TooltipHost>,
    active_context_menu_invoker: Option<SharedString>,
}

impl BottomStatusBarView {
    pub(in super::super) fn new(
        theme: AppTheme,
        root_view: WeakEntity<GitCometView>,
        tooltip_host: WeakEntity<TooltipHost>,
    ) -> Self {
        Self {
            theme,
            root_view,
            tooltip_host,
            active_context_menu_invoker: None,
        }
    }

    pub(in super::super) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;
        cx.notify();
    }

    pub(in super::super) fn set_active_context_menu_invoker(
        &mut self,
        next: Option<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.active_context_menu_invoker == next {
            return;
        }

        self.active_context_menu_invoker = next;
        cx.notify();
    }

    fn set_tooltip_text_if_changed(
        &mut self,
        next: Option<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self
            .tooltip_host
            .update(cx, |host, cx| host.set_tooltip_text_if_changed(next, cx));
    }

    fn clear_tooltip_if_matches(&mut self, tooltip: &SharedString, cx: &mut gpui::Context<Self>) {
        let tooltip = tooltip.clone();
        let _ = self
            .tooltip_host
            .update(cx, |host, cx| host.clear_tooltip_if_matches(&tooltip, cx));
    }

    fn open_popover_for_bounds(
        &mut self,
        kind: PopoverKind,
        anchor_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.open_popover_for_bounds(kind, anchor_bounds, window, cx);
        });
    }

    fn activate_context_menu_invoker(
        &mut self,
        invoker: SharedString,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, move |root, cx| {
            root.set_active_context_menu_invoker(Some(invoker), cx);
        });
    }
}

impl Render for BottomStatusBarView {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let ui_scale_percent = crate::ui_scale::current(cx).percent;
        let scaled_px =
            |value: f32| crate::ui_scale::design_px_from_percent(value, ui_scale_percent);
        let zoom_picker_invoker: SharedString = "ui_scale_picker".into();
        let zoom_picker_active = self
            .active_context_menu_invoker
            .as_ref()
            .is_some_and(|id| id.as_ref() == zoom_picker_invoker.as_ref());
        let zoom_button_bg =
            with_alpha(theme.colors.accent, if theme.is_dark { 0.26 } else { 0.20 });
        let zoom_label = if ui_scale_percent == crate::ui_scale::DEFAULT_UI_SCALE_PERCENT {
            String::new()
        } else {
            crate::ui_scale::label(ui_scale_percent)
        };

        let zoom_icon_color = if zoom_picker_active {
            theme.colors.accent
        } else {
            theme.colors.text_muted
        };
        let zoom_button = components::Button::new("bottom_status_bar_zoom", zoom_label)
            .start_slot(
                div()
                    .debug_selector(|| "bottom_status_bar_zoom_icon".to_string())
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(svg_icon("icons/zoom.svg", zoom_icon_color, scaled_px(14.0))),
            )
            .style(components::ButtonStyle::Subtle)
            .borderless()
            .no_hover_border()
            .selected(zoom_picker_active)
            .selected_bg(zoom_button_bg)
            .on_click_with_bounds(theme, cx, move |this, _e, bounds, window, cx| {
                this.activate_context_menu_invoker(zoom_picker_invoker.clone(), cx);
                this.open_popover_for_bounds(PopoverKind::UiScalePicker, bounds, window, cx);
            })
            .on_hover(cx.listener(|this, hovering: &bool, _w, cx| {
                let tooltip: SharedString = "Adjust zoom".into();
                if *hovering {
                    this.set_tooltip_text_if_changed(Some(tooltip), cx);
                } else {
                    this.clear_tooltip_if_matches(&tooltip, cx);
                }
            }))
            .debug_selector(|| "bottom_status_bar_zoom".to_string());

        div()
            .id("bottom_status_bar")
            .w_full()
            .h(components::Tab::container_height(ui_scale_percent))
            .flex_none()
            .flex()
            .items_center()
            .justify_end()
            .px_2()
            .bg(theme.colors.surface_bg)
            .border_t_1()
            .border_color(theme.colors.border)
            .child(zoom_button)
    }
}
