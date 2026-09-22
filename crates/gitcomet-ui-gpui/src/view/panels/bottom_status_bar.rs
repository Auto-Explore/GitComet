use super::*;
use crate::kit::interaction as controls;
use crate::view::components::{ControlInteractionExt, InteractionState, InteractionStyle};

/// Slimmer than the tab-bar slot the bottom bar used to borrow; it hosts the
/// pane collapse toggles, the zoom control and the branding strip on one shared
/// centerline, so every saved pixel goes to the content area.
const BOTTOM_STATUS_BAR_HEIGHT_PX: f32 = 26.0;
/// The bar and its chips are chrome like the title bar, so they take the
/// density ramp.
const BOTTOM_STATUS_BAR_COMFORTABLE_HEIGHT_PX: f32 = 34.0;
const BOTTOM_STATUS_BAR_ITEM_HEIGHT_PX: f32 = 18.0;
const BOTTOM_STATUS_BAR_ITEM_COMFORTABLE_HEIGHT_PX: f32 = 26.0;
const PANE_TOGGLE_ICON_SIZE_PX: f32 = 16.0;

fn pro_launch_label(today: jiff::civil::Date) -> SharedString {
    let launch_date = jiff::civil::date(2026, 10, 7);
    let days = today.duration_until(launch_date).as_secs() / 86_400;
    match days {
        1 => "Pro launches in 1 day".into(),
        2..=100 => format!("Pro launches in {days} days").into(),
        _ => "Get Pro!".into(),
    }
}

fn sidebar_toggle_icon_path(collapsed: bool) -> &'static str {
    if collapsed {
        "icons/side_panel_left_expand.svg"
    } else {
        "icons/side_panel_left.svg"
    }
}

fn details_toggle_icon_path(collapsed: bool) -> &'static str {
    if collapsed {
        "icons/side_panel_right_expand.svg"
    } else {
        "icons/side_panel_right.svg"
    }
}

/// Shared shape for the branding links on the bar's trailing end. No plate and
/// no outline — beside the wordmark and the version number these read as links,
/// and a badge each would turn the corner into a row of buttons. Hover is
/// carried entirely by the supplied tint, which the Discord glyph picks up through
/// `group_hover` on this element's group.
fn status_bar_chip(
    id: &'static str,
    hover_color: gpui::Rgba,
    ui_scale_percent: u32,
    theme: AppTheme,
) -> gpui::Stateful<gpui::Div> {
    let scaled_px = crate::ui_scale::scaler(ui_scale_percent);

    div()
        .id(id)
        .group(id)
        .debug_selector(move || id.to_string())
        .h(scaled_px(theme.metrics.row_height(
            BOTTOM_STATUS_BAR_ITEM_HEIGHT_PX,
            BOTTOM_STATUS_BAR_ITEM_COMFORTABLE_HEIGHT_PX,
        )))
        .px(scaled_px(4.0))
        .flex()
        .items_center()
        .justify_center()
        .cursor(CursorStyle::PointingHand)
        .tab_index(0)
        .control_interaction(
            InteractionStyle::link(theme)
                .hover(StyleRefinement::default().text_color(hover_color))
                .pressed(StyleRefinement::default().text_color(hover_color)),
            InteractionState::default(),
        )
}

pub(in super::super) struct BottomStatusBarView {
    theme: AppTheme,
    state: Arc<AppState>,
    _ui_model_subscription: gpui::Subscription,
    root_view: WeakEntity<GitCometView>,
    active_context_menu_invoker: Option<SharedString>,
    minimized_hook_activity_repos: rustc_hash::FxHashSet<RepoId>,
    pro_launch_label: SharedString,
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
            minimized_hook_activity_repos: Default::default(),
            // Use local calendar days and keep the startup label for this window.
            pro_launch_label: pro_launch_label(jiff::Zoned::now().date()),
        }
    }

    fn hook_activity_summary(state: &AppState) -> (Option<RepoId>, usize, bool) {
        let repo_id = state.active_repo;
        let (active, warning) = repo_id
            .and_then(|repo_id| state.repos.iter().find(|repo| repo.id == repo_id))
            .map(|repo| {
                (
                    repo.feedback
                        .hook_activity
                        .iter()
                        .filter(|operation| operation.is_reportable() && operation.status.is_active())
                        .count(),
                    repo.feedback.hook_activity.iter().rev().any(|operation| {
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

    pub(in super::super) fn set_minimized_hook_activity_repos(
        &mut self,
        repos: &rustc_hash::FxHashSet<RepoId>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.minimized_hook_activity_repos != *repos {
            self.minimized_hook_activity_repos.clone_from(repos);
            cx.notify();
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
        kind: impl Into<PopoverRequest>,
        anchor_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let kind: PopoverRequest = kind.into();
        let _ = self.root_view.update(cx, |root, cx| {
            root.open_popover_for_bounds(kind, anchor_bounds, window, cx);
        });
    }

    fn open_popover_centered(
        &mut self,
        kind: impl Into<PopoverRequest>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let kind: PopoverRequest = kind.into();
        let _ = self.root_view.update(cx, |root, cx| {
            root.open_popover_centered(kind, window, cx);
        });
    }
}

impl Render for BottomStatusBarView {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let ui_scale_percent = crate::ui_scale::current(cx).percent;
        let scaled_px = crate::ui_scale::scaler(ui_scale_percent);
        let zoom_picker_invoker: SharedString = "ui_scale_picker".into();
        let zoom_picker_active = self
            .active_context_menu_invoker
            .as_ref()
            .is_some_and(|id| id.as_ref() == zoom_picker_invoker.as_ref());
        let zoom_button_bg = components::control_open_background(theme);
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
            .open(zoom_picker_active)
            .selected_bg(zoom_button_bg)
            .on_click_with_bounds(theme, cx, move |this, _e, bounds, window, cx| {
                this.open_popover_for_bounds(
                    PopoverKind::UiScalePicker.invoked_by(zoom_picker_invoker.clone()),
                    bounds,
                    window,
                    cx,
                );
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

        // These footer toggles convey panel visibility through their icons and
        // tooltips, with ordinary button feedback and no persistent highlight.
        let sidebar_toggle = components::Button::new("sidebar_toggle", "")
            .start_slot(svg_icon(
                sidebar_toggle_icon_path(sidebar_collapsed),
                theme.colors.foreground.secondary,
                scaled_px(PANE_TOGGLE_ICON_SIZE_PX),
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
                details_toggle_icon_path(details_collapsed),
                theme.colors.foreground.secondary,
                scaled_px(PANE_TOGGLE_ICON_SIZE_PX),
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
        let keep_minimized =
            active_repo_id.is_some_and(|id| self.minimized_hook_activity_repos.contains(&id));
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
                        .text_size(theme.ui_text(9.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.colors.accent.foreground)
                        .child(active_hook_count.to_string()),
                )
            })
            .when(has_hook_warning && active_hook_count == 0, |icon| {
                icon.debug_selector(|| "bottom_hook_activity_warning".to_string())
            });
        let hook_activity_button = components::Button::new("bottom_hook_activity", "")
            .selected(keep_minimized)
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
            .gitcomet_tooltip(
                theme,
                if keep_minimized {
                    "Git hook activity — keep minimized enabled".into()
                } else {
                    "Git hook activity".into()
                },
            )
            .debug_selector(|| "bottom_hook_activity".to_string());

        // Branding strip: the edition badge moved down here from the title bar,
        // where it crowded the repository tabs.
        let discord_badge = status_bar_chip(
            "bottom_status_bar_discord",
            theme.colors.accent.foreground,
            ui_scale_percent,
            theme,
        )
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
        .on_activate(
            false,
            controls::ControlActivation::Action,
            cx.listener(|_this, _e: &ClickEvent, _window, cx| {
                cx.stop_propagation();
                cx.open_url(DISCORD_URL);
            }),
        )
        .gitcomet_tooltip(theme, "Join the GitComet Discord".into());

        let free_badge = status_bar_chip(
            "bottom_status_bar_free_badge",
            theme.colors.accent.foreground,
            ui_scale_percent,
            theme,
        )
        .text_size(theme.ui_text(11.0))
        .line_height(scaled_px(theme.metrics.ui_text(12.0)))
        .font_weight(FontWeight::NORMAL)
        .text_color(with_alpha(
            theme.colors.foreground.primary,
            if theme.is_dark { 0.72 } else { 0.62 },
        ))
        .on_activate(
            false,
            controls::ControlActivation::Action,
            cx.listener(|_this, _e: &ClickEvent, _window, cx| {
                cx.stop_propagation();
                cx.open_url(EDITIONS_URL);
            }),
        )
        .gitcomet_tooltip(theme, "See GitComet editions".into())
        .child("FREE");

        let pro_link = status_bar_chip(
            "bottom_status_bar_pro_link",
            theme.colors.accent.foreground,
            ui_scale_percent,
            theme,
        )
        .text_size(theme.ui_text(11.0))
        .line_height(scaled_px(theme.metrics.ui_text(12.0)))
        .text_color(theme.colors.foreground.secondary)
        .on_activate(
            false,
            controls::ControlActivation::Action,
            cx.listener(|_this, _e: &ClickEvent, _window, cx| {
                cx.stop_propagation();
                cx.open_url(EDITIONS_URL);
            }),
        )
        .gitcomet_tooltip(theme, "See GitComet Pro".into())
        .child(self.pro_launch_label.clone());

        // GPUI paints an SVG as a mask tinted by the text color, so the mark's
        // own brand blue never reaches the screen. Apply that blue explicitly
        // so the mark keeps its brand color regardless of the theme.
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
            .tab_index(0)
            .control_interaction(InteractionStyle::link(theme), InteractionState::default())
            .child(svg_icon(
                "icons/gitcomet_mark.svg",
                gpui::rgb(0x5ac1fe),
                scaled_px(13.0),
            ))
            .child(
                div()
                    .debug_selector(|| "bottom_status_bar_brand".to_string())
                    .text_size(theme.ui_text(11.0))
                    .line_height(scaled_px(theme.metrics.ui_text(12.0)))
                    .child("GitComet"),
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|_this, _e: &ClickEvent, _window, cx| {
                    cx.stop_propagation();
                    cx.open_url(WEBSITE_URL);
                }),
            )
            .gitcomet_tooltip(theme, "Open gitcomet.dev".into());

        let version_label: SharedString = format!("v{}", env!("CARGO_PKG_VERSION")).into();
        let version_link = div()
            .id("bottom_status_bar_version")
            .debug_selector(|| "bottom_status_bar_version".to_string())
            .flex()
            .items_center()
            .cursor(CursorStyle::PointingHand)
            .text_size(theme.ui_text(11.0))
            .line_height(scaled_px(theme.metrics.ui_text(12.0)))
            .text_color(theme.colors.foreground.secondary)
            .tab_index(0)
            .control_interaction(InteractionStyle::link(theme), InteractionState::default())
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|_this, _e: &ClickEvent, _window, cx| {
                    cx.stop_propagation();
                    cx.open_url(RELEASES_URL);
                }),
            )
            .gitcomet_tooltip(theme, "View GitComet releases".into())
            .child(version_label);

        div()
            .id("bottom_status_bar")
            .w_full()
            .h(scaled_px(theme.metrics.row_height(
                BOTTOM_STATUS_BAR_HEIGHT_PX,
                BOTTOM_STATUS_BAR_COMFORTABLE_HEIGHT_PX,
            )))
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
                            .child(pro_link)
                            .child(brand)
                            .child(version_link),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{details_toggle_icon_path, pro_launch_label, sidebar_toggle_icon_path};

    #[test]
    fn pro_countdown_uses_calendar_days_and_falls_back_outside_launch_window() {
        use jiff::civil::date;

        for (today, expected) in [
            (date(2026, 9, 7), "Pro launches in 30 days"),
            (date(2026, 9, 8), "Pro launches in 29 days"),
            (date(2026, 10, 6), "Pro launches in 1 day"),
            (date(2026, 10, 7), "Get Pro!"),
            (date(2026, 10, 8), "Get Pro!"),
            (date(2026, 6, 29), "Pro launches in 100 days"),
            (date(2026, 6, 28), "Get Pro!"),
            (date(2025, 10, 7), "Get Pro!"),
        ] {
            assert_eq!(pro_launch_label(today).as_ref(), expected, "{today}");
        }
    }

    #[test]
    fn pane_toggle_arrows_only_appear_when_the_panel_is_collapsed() {
        assert_eq!(sidebar_toggle_icon_path(false), "icons/side_panel_left.svg");
        assert_eq!(
            sidebar_toggle_icon_path(true),
            "icons/side_panel_left_expand.svg"
        );
        assert_eq!(
            details_toggle_icon_path(false),
            "icons/side_panel_right.svg"
        );
        assert_eq!(
            details_toggle_icon_path(true),
            "icons/side_panel_right_expand.svg"
        );
    }
}
