use super::*;

/// Slimmer than the tab-bar slot the bottom bar used to borrow; it hosts the
/// pane collapse toggles, the zoom control and the branding strip on one shared
/// centerline, so every saved pixel goes to the content area.
const BOTTOM_STATUS_BAR_HEIGHT_PX: f32 = 26.0;

/// Shared shape for the branding links on the bar's trailing end. No plate and
/// no outline — beside the wordmark and the version number these read as links,
/// and a badge each would turn the corner into a row of buttons. Hover is
/// carried entirely by the accent tint, which the Discord glyph picks up through
/// `group_hover` on this element's group.
fn status_bar_chip(
    id: &'static str,
    theme: AppTheme,
    ui_scale_percent: u32,
) -> gpui::Stateful<gpui::Div> {
    let scaled_px = |value: f32| crate::ui_scale::design_px_from_percent(value, ui_scale_percent);

    div()
        .id(id)
        .group(id)
        .debug_selector(move || id.to_string())
        .h(scaled_px(18.0))
        .px(scaled_px(4.0))
        .flex()
        .items_center()
        .justify_center()
        .cursor(CursorStyle::PointingHand)
        .hover(move |s| s.text_color(theme.colors.accent.foreground))
        .active(move |s| s.text_color(theme.colors.accent.foreground))
}

pub(in super::super) struct BottomStatusBarView {
    theme: AppTheme,
    state: Arc<AppState>,
    _ui_model_subscription: gpui::Subscription,
    root_view: WeakEntity<GitCometView>,
    active_context_menu_invoker: Option<SharedString>,
}

impl BottomStatusBarView {
    pub(in super::super) fn new(
        theme: AppTheme,
        ui_model: Entity<AppUiModel>,
        root_view: WeakEntity<GitCometView>,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let state = Arc::clone(&ui_model.read(cx).state);
        let subscription = cx.observe(&ui_model, |this, model, cx| {
            let previous_summary = Self::hook_activity_summary(&this.state);
            let next = Arc::clone(&model.read(cx).state);
            let next_summary = Self::hook_activity_summary(&next);
            this.state = next;
            if next_summary != previous_summary {
                cx.notify();
            }
        });
        Self {
            theme,
            state,
            _ui_model_subscription: subscription,
            root_view,
            active_context_menu_invoker: None,
        }
    }

    fn hook_activity_summary(state: &AppState) -> (Option<RepoId>, usize, bool) {
        let repo_id = state.active_repo;
        let (active, warning) = repo_id
            .and_then(|repo_id| state.repos.iter().find(|repo| repo.id == repo_id))
            .map(|repo| {
                (
                    repo.hook_activity
                        .iter()
                        .filter(|operation| operation.has_hooks() && operation.status.is_active())
                        .count(),
                    repo.hook_activity.iter().rev().any(|operation| {
                        matches!(
                            operation.status,
                            GitHookOperationStatus::SucceededWithHookFailure
                                | GitHookOperationStatus::Failed
                                | GitHookOperationStatus::TimedOut
                        )
                    }),
                )
            })
            .unwrap_or((0, false));
        (repo_id, active, warning)
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

    fn open_popover_centered(
        &mut self,
        kind: PopoverKind,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.open_popover_centered(kind, window, cx);
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
        let zoom_button_bg = with_alpha(
            theme.colors.accent.foreground,
            if theme.is_dark { 0.26 } else { 0.20 },
        );
        let zoom_label = if ui_scale_percent == crate::ui_scale::DEFAULT_UI_SCALE_PERCENT {
            String::new()
        } else {
            crate::ui_scale::label(ui_scale_percent)
        };

        let zoom_icon_color = if zoom_picker_active {
            theme.colors.accent.foreground
        } else {
            theme.colors.foreground.secondary
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
                theme.colors.foreground.secondary,
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
                theme.colors.foreground.secondary,
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

        let (active_repo_id, active_hook_count, has_hook_warning) =
            Self::hook_activity_summary(&self.state);
        let activity_icon_color = if active_hook_count > 0 {
            theme.colors.accent.foreground
        } else if has_hook_warning {
            theme.colors.status.warning.foreground
        } else {
            theme.colors.foreground.secondary
        };
        let activity_icon = div()
            .flex()
            .items_center()
            .gap(scaled_px(3.0))
            .child(
                div()
                    .debug_selector(|| "bottom_hook_activity_lightning".to_string())
                    .child(svg_icon(
                        "icons/lightning.svg",
                        activity_icon_color,
                        scaled_px(13.0),
                    )),
            )
            .when(active_hook_count > 0, |icon| {
                icon.child(
                    div()
                        .debug_selector(|| "bottom_hook_activity_running".to_string())
                        .min_w(scaled_px(14.0))
                        .h(scaled_px(14.0))
                        .px(scaled_px(3.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(scaled_px(999.0))
                        .bg(with_alpha(
                            theme.colors.accent.foreground,
                            if theme.is_dark { 0.24 } else { 0.16 },
                        ))
                        .text_size(scaled_px(9.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.colors.accent.foreground)
                        .child(active_hook_count.to_string()),
                )
            })
            .when(has_hook_warning && active_hook_count == 0, |icon| {
                icon.debug_selector(|| "bottom_hook_activity_warning".to_string())
            });
        let hook_activity_button = components::Button::new("bottom_hook_activity", "")
            .start_slot(activity_icon)
            .style(components::ButtonStyle::Subtle)
            .borderless()
            .disabled(active_repo_id.is_none())
            .on_click(theme, cx, move |this, _e, window, cx| {
                let Some(repo_id) = active_repo_id else {
                    return;
                };
                this.open_popover_centered(
                    PopoverKind::HookActivity {
                        repo_id,
                        operation_id: None,
                    },
                    window,
                    cx,
                );
            })
            .gitcomet_tooltip(theme, "Git hook activity".into())
            .debug_selector(|| "bottom_hook_activity".to_string());

        // Branding strip: the edition badge moved down here from the title bar,
        // where it crowded the repository tabs.
        let discord_badge = status_bar_chip("bottom_status_bar_discord", theme, ui_scale_percent)
            .child(
                gpui::svg()
                    .path("icons/discord.svg")
                    .w(scaled_px(12.0))
                    .h(scaled_px(12.0))
                    .flex_shrink_0()
                    .text_color(theme.colors.foreground.secondary)
                    .group_hover("bottom_status_bar_discord", move |s| {
                        s.text_color(theme.colors.accent.foreground)
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
                theme.colors.foreground.primary,
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
        //
        // The color lives on the link itself and the wordmark inherits it, so
        // one `.hover()` on this stateful element tints the text. A style on the
        // stateless child could not do it: gpui only repaints on hover for
        // elements that carry state, so the child's own hover would compute a
        // tint that never reaches the screen.
        let brand = div()
            .id("bottom_status_bar_brand_link")
            .debug_selector(|| "bottom_status_bar_brand_link".to_string())
            .flex()
            .items_center()
            .gap(scaled_px(4.0))
            .cursor(CursorStyle::PointingHand)
            .text_color(theme.colors.foreground.secondary)
            .hover(move |s| s.text_color(theme.colors.accent.foreground))
            .active(move |s| s.text_color(theme.colors.accent.foreground))
            .child(svg_icon(
                "icons/gitcomet_mark.svg",
                theme.colors.accent.foreground,
                scaled_px(13.0),
            ))
            .child(
                div()
                    .debug_selector(|| "bottom_status_bar_brand".to_string())
                    .text_size(scaled_px(11.0))
                    .line_height(scaled_px(12.0))
                    .child("GitComet"),
            )
            .on_click(cx.listener(|_this, _e: &ClickEvent, _window, cx| {
                cx.stop_propagation();
                cx.open_url(WEBSITE_URL);
            }))
            .gitcomet_tooltip(theme, "Open gitcomet.dev".into());

        let version_label: SharedString = format!("v{}", env!("CARGO_PKG_VERSION")).into();
        let version_link = div()
            .id("bottom_status_bar_version")
            .debug_selector(|| "bottom_status_bar_version".to_string())
            .flex()
            .items_center()
            .cursor(CursorStyle::PointingHand)
            .text_size(scaled_px(11.0))
            .line_height(scaled_px(12.0))
            .text_color(theme.colors.foreground.secondary)
            .hover(move |s| s.text_color(theme.colors.accent.foreground))
            .active(move |s| s.text_color(theme.colors.accent.foreground))
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
            .bg(theme.colors.surface.chrome)
            .when_some(
                crate::view::chrome::client_frame_corner_rounding(theme, window),
                |d, rounding| {
                    d.when(rounding.bottom_left, |d| d.rounded_bl(rounding.radius))
                        .when(rounding.bottom_right, |d| d.rounded_br(rounding.radius))
                },
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(scaled_px(2.0))
                    .child(sidebar_toggle),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(scaled_px(2.0))
                    .child(details_toggle)
                    .child(hook_activity_button)
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
