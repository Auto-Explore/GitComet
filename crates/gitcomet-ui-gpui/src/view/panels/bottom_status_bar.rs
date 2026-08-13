use super::*;

/// Slimmer than the tab-bar slot the bottom bar used to borrow; it hosts the
/// pane collapse toggles, the zoom control and the branding strip on one shared
/// centerline, so every saved pixel goes to the content area.
const BOTTOM_STATUS_BAR_HEIGHT_PX: f32 = 26.0;

/// Pill worn by the branding chips on the bar's trailing end. Same chrome the
/// edition badge had while it lived in the title bar, so every chip here reads
/// as one family.
fn status_bar_chip(
    id: &'static str,
    theme: AppTheme,
    ui_scale_percent: u32,
) -> gpui::Stateful<gpui::Div> {
    let scaled_px = |value: f32| crate::ui_scale::design_px_from_percent(value, ui_scale_percent);
    let bg = with_alpha(
        theme.colors.text_muted,
        if theme.is_dark { 0.22 } else { 0.16 },
    );
    let border = with_alpha(
        theme.colors.text_muted,
        if theme.is_dark { 0.34 } else { 0.28 },
    );
    let hover_bg = with_alpha(theme.colors.accent, if theme.is_dark { 0.18 } else { 0.12 });
    let hover_border = with_alpha(theme.colors.accent, if theme.is_dark { 0.42 } else { 0.34 });
    let active_bg = with_alpha(theme.colors.accent, if theme.is_dark { 0.28 } else { 0.20 });
    let active_border = with_alpha(theme.colors.accent, if theme.is_dark { 0.58 } else { 0.46 });

    div()
        .id(id)
        .group(id)
        .debug_selector(move || id.to_string())
        .h(scaled_px(18.0))
        .px(scaled_px(6.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme.radii.pill))
        .cursor(CursorStyle::PointingHand)
        .bg(bg)
        .border_1()
        .border_color(border)
        .hover(move |s| {
            s.bg(hover_bg)
                .border_color(hover_border)
                .text_color(theme.colors.accent)
        })
        .active(move |s| {
            s.bg(active_bg)
                .border_color(active_border)
                .text_color(theme.colors.accent)
        })
}

pub(in super::super) struct BottomStatusBarView {
    theme: AppTheme,
    root_view: WeakEntity<GitCometView>,
    active_context_menu_invoker: Option<SharedString>,
}

impl BottomStatusBarView {
    pub(in super::super) fn new(theme: AppTheme, root_view: WeakEntity<GitCometView>) -> Self {
        Self {
            theme,
            root_view,
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
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
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
                    .child(svg_icon(
                        "icons/zoom_in.svg",
                        zoom_icon_color,
                        scaled_px(14.0),
                    )),
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
            .gitcomet_tooltip(theme, "Adjust zoom".into())
            .debug_selector(|| "bottom_status_bar_zoom".to_string());

        // Pane collapse toggles live here (not floating inside the panes) so
        // they share one centerline with the zoom control.
        let (sidebar_collapsed, details_collapsed) = self
            .root_view
            .upgrade()
            .map(|view| {
                let root = view.read(cx);
                (root.sidebar_collapsed, root.details_collapsed)
            })
            .unwrap_or((false, false));

        let sidebar_toggle = components::Button::new("sidebar_toggle", "")
            .start_slot(svg_icon(
                if sidebar_collapsed {
                    "icons/arrow_right.svg"
                } else {
                    "icons/arrow_left.svg"
                },
                theme.colors.text_muted,
                scaled_px(12.0),
            ))
            .style(components::ButtonStyle::Transparent)
            .on_click(theme, cx, |this, _e, _w, cx| {
                let _ = this.root_view.update(cx, |root, cx| {
                    root.set_sidebar_collapsed(!root.sidebar_collapsed, cx);
                });
            })
            .gitcomet_tooltip(
                theme,
                if sidebar_collapsed {
                    "Show sidebar".into()
                } else {
                    "Hide sidebar".into()
                },
            );

        let details_toggle = components::Button::new("details_toggle", "")
            .start_slot(svg_icon(
                if details_collapsed {
                    "icons/arrow_left.svg"
                } else {
                    "icons/arrow_right.svg"
                },
                theme.colors.text_muted,
                scaled_px(12.0),
            ))
            .style(components::ButtonStyle::Transparent)
            .on_click(theme, cx, |this, _e, _w, cx| {
                let _ = this.root_view.update(cx, |root, cx| {
                    root.set_details_collapsed(!root.details_collapsed, cx);
                });
            })
            .gitcomet_tooltip(
                theme,
                if details_collapsed {
                    "Show details panel".into()
                } else {
                    "Hide details panel".into()
                },
            );

        // Branding strip: the edition badge moved down here from the title bar,
        // where it crowded the repository tabs.
        let discord_badge = status_bar_chip("bottom_status_bar_discord", theme, ui_scale_percent)
            .child(
                gpui::svg()
                    .path("icons/discord.svg")
                    .w(scaled_px(12.0))
                    .h(scaled_px(12.0))
                    .flex_shrink_0()
                    .text_color(theme.colors.text_muted)
                    .group_hover("bottom_status_bar_discord", move |s| {
                        s.text_color(theme.colors.accent)
                    }),
            )
            .on_click(cx.listener(|_this, _e: &ClickEvent, _window, cx| {
                cx.stop_propagation();
                cx.open_url(DISCORD_URL);
            }))
            .gitcomet_tooltip(theme, "Join the GitComet Discord".into());

        let free_badge = status_bar_chip("bottom_status_bar_free_badge", theme, ui_scale_percent)
            .text_size(scaled_px(11.0))
            .line_height(scaled_px(12.0))
            .font_weight(FontWeight::NORMAL)
            .text_color(with_alpha(
                theme.colors.text,
                if theme.is_dark { 0.72 } else { 0.62 },
            ))
            .on_click(cx.listener(|_this, _e: &ClickEvent, _window, cx| {
                cx.stop_propagation();
                cx.open_url(EDITIONS_URL);
            }))
            .gitcomet_tooltip(theme, "See GitComet editions".into())
            .child("FREE");

        // GPUI paints an SVG as a mask tinted by the text color, so the mark's
        // own brand blue never reaches the screen — an untinted mark renders
        // invisible. Tint it with the accent so it stays legible in every theme.
        let brand = div()
            .flex()
            .items_center()
            .gap(scaled_px(4.0))
            .child(svg_icon(
                "icons/gitcomet_mark.svg",
                theme.colors.accent,
                scaled_px(13.0),
            ))
            .child(
                div()
                    .debug_selector(|| "bottom_status_bar_brand".to_string())
                    .text_size(scaled_px(11.0))
                    .line_height(scaled_px(12.0))
                    .text_color(theme.colors.text_muted)
                    .child("GitComet"),
            );

        let version_label: SharedString = format!("v{}", env!("CARGO_PKG_VERSION")).into();
        let version_link = div()
            .id("bottom_status_bar_version")
            .debug_selector(|| "bottom_status_bar_version".to_string())
            .flex()
            .items_center()
            .cursor(CursorStyle::PointingHand)
            .text_size(scaled_px(11.0))
            .line_height(scaled_px(12.0))
            .text_color(theme.colors.text_muted)
            .hover(move |s| s.text_color(theme.colors.accent))
            .active(move |s| s.text_color(theme.colors.accent))
            .on_click(cx.listener(|_this, _e: &ClickEvent, _window, cx| {
                cx.stop_propagation();
                cx.open_url(RELEASES_URL);
            }))
            .gitcomet_tooltip(theme, "View GitComet releases".into())
            .child(version_label);

        div()
            .id("bottom_status_bar")
            .w_full()
            .h(scaled_px(BOTTOM_STATUS_BAR_HEIGHT_PX))
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .px_2()
            .bg(theme.colors.sidebar_bg)
            .when_some(
                crate::view::chrome::client_frame_corner_rounding(theme, window),
                |d, rounding| {
                    d.when(rounding.bottom_left, |d| d.rounded_bl(rounding.radius))
                        .when(rounding.bottom_right, |d| d.rounded_br(rounding.radius))
                },
            )
            .child(sidebar_toggle)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(scaled_px(2.0))
                    .child(details_toggle)
                    .child(zoom_button)
                    .child(
                        // Branding chips want more air between them than the
                        // toggles, which read as one control group.
                        div()
                            .flex()
                            .items_center()
                            .gap(scaled_px(6.0))
                            .pl(scaled_px(6.0))
                            .child(discord_badge)
                            .child(free_badge)
                            .child(brand)
                            .child(version_link),
                    ),
            )
    }
}
