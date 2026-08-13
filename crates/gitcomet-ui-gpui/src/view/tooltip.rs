use super::*;
#[cfg(test)]
use std::cell::RefCell;

#[cfg(test)]
thread_local! {
    static VISIBLE_TOOLTIP_TEXT_FOR_TEST: RefCell<Option<SharedString>> = const { RefCell::new(None) };
}

/// Bumped on every mouse-down (see [`dismiss_tooltips_on_mouse_down`]); a
/// visible tooltip bubble compares against the epoch it was built at and
/// renders empty once the epoch moves on, so clicks always hide tooltips.
#[derive(Default)]
pub(super) struct TooltipDismissEpoch(u64);

impl gpui::Global for TooltipDismissEpoch {}

/// True while a popover / context menu is open. The overlay's anchor stays
/// hovered after the opening click, so its tooltip would re-show painted on
/// top of the open surface; bubbles render empty while this is set.
#[derive(Default)]
pub(super) struct TooltipOverlaySuppression(bool);

impl gpui::Global for TooltipOverlaySuppression {}

pub(super) fn set_tooltips_suppressed_by_overlay(open: bool, cx: &mut App) {
    if tooltips_suppressed_by_overlay(cx) != open {
        cx.set_global(TooltipOverlaySuppression(open));
    }
}

fn tooltips_suppressed_by_overlay(cx: &App) -> bool {
    cx.try_global::<TooltipOverlaySuppression>()
        .is_some_and(|state| state.0)
}

pub(super) fn current_tooltip_dismiss_epoch(cx: &App) -> u64 {
    cx.try_global::<TooltipDismissEpoch>()
        .map(|epoch| epoch.0)
        .unwrap_or(0)
}

/// Hides every visible gpui-managed tooltip bubble. Registered on window
/// roots via `capture_any_mouse_down` so it runs for clicks anywhere.
pub(super) fn dismiss_tooltips_on_mouse_down(cx: &mut App) {
    let next = current_tooltip_dismiss_epoch(cx).wrapping_add(1);
    cx.set_global(TooltipDismissEpoch(next));
}

pub(super) trait GitCometTooltipExt: gpui::StatefulInteractiveElement + Sized {
    fn gitcomet_tooltip(self, theme: AppTheme, text: SharedString) -> Self {
        self.tooltip(move |_window, cx| {
            let epoch = current_tooltip_dismiss_epoch(cx);
            AnyView::from(cx.new(|cx| {
                let epoch_observer = cx.observe_global::<TooltipDismissEpoch>(|_, cx| cx.notify());
                let overlay_observer =
                    cx.observe_global::<TooltipOverlaySuppression>(|_, cx| cx.notify());
                TooltipBubbleView {
                    theme,
                    text: text.clone(),
                    epoch,
                    _epoch_observer: epoch_observer,
                    _overlay_observer: overlay_observer,
                }
            }))
        })
    }
}

impl<T: gpui::StatefulInteractiveElement> GitCometTooltipExt for T {}

struct TooltipBubbleView {
    theme: AppTheme,
    text: SharedString,
    /// Dismiss epoch at build time; a later epoch means a click happened
    /// while this bubble was up, so it must disappear.
    epoch: u64,
    _epoch_observer: gpui::Subscription,
    _overlay_observer: gpui::Subscription,
}

impl Render for TooltipBubbleView {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        if current_tooltip_dismiss_epoch(cx) != self.epoch {
            return div();
        }
        if tooltips_suppressed_by_overlay(cx) {
            return div();
        }

        #[cfg(test)]
        VISIBLE_TOOLTIP_TEXT_FOR_TEST.with(|value| {
            value.replace(Some(self.text.clone()));
        });

        div().pl(px(11.0)).pt(px(17.0)).child(
            div()
                .px_2()
                .py_1()
                .bg(self.theme.colors.tooltip_bg)
                .rounded(px(self.theme.radii.row))
                .shadow(crate::theme::shadow_popover(self.theme))
                .text_xs()
                .text_color(self.theme.colors.tooltip_text)
                .child(self.text.clone()),
        )
    }
}

#[cfg(test)]
pub(super) fn clear_visible_tooltip_text_for_test() {
    VISIBLE_TOOLTIP_TEXT_FOR_TEST.with(|value| {
        value.replace(None);
    });
}

#[cfg(test)]
pub(super) fn tooltip_text_for_test() -> Option<SharedString> {
    VISIBLE_TOOLTIP_TEXT_FOR_TEST.with(|value| value.borrow().clone())
}

impl GitCometView {
    pub(super) fn schedule_ui_settings_persist(&mut self, cx: &mut gpui::Context<Self>) {
        if !crate::ui_runtime::current().persists_ui_settings() {
            let _ = cx;
            return;
        }

        self.ui_settings_persist_seq = self.ui_settings_persist_seq.wrapping_add(1);
        let seq = self.ui_settings_persist_seq;

        cx.spawn(
            async move |view: WeakEntity<GitCometView>, cx: &mut gpui::AsyncApp| {
                smol::Timer::after(Duration::from_millis(250)).await;
                let settings = view
                    .update(cx, |this, cx| {
                        if this.ui_settings_persist_seq != seq {
                            return None;
                        }

                        let ww: f32 = this.last_window_size.width.round().into();
                        let wh: f32 = this.last_window_size.height.round().into();
                        let window_width = (ww.is_finite() && ww >= 1.0).then_some(ww as u32);
                        let window_height = (wh.is_finite() && wh >= 1.0).then_some(wh as u32);

                        let (
                            history_show_graph,
                            history_show_author,
                            history_show_date,
                            history_show_sha,
                        ) = this
                            .main_pane
                            .read(cx)
                            .history_visible_column_preferences(cx);
                        let (history_show_tags, history_auto_fetch_tags_on_repo_activation) = this
                            .main_pane
                            .read(cx)
                            .history_tag_preferences(cx);
                        let history_relative_dates =
                            this.main_pane.read(cx).history_relative_dates(cx);
                        let (
                            mergetool_auto_advance,
                            mergetool_collapse_unchanged,
                            mergetool_output_scroll_sync,
                            mergetool_show_line_numbers,
                        ) = this.main_pane.read(cx).mergetool_preferences();
                        let mergetool_view_three_way =
                            this.main_pane.read(cx).mergetool_view_three_way;
                        let (change_tracking_height, untracked_height) =
                            this.details_pane.read(cx).saved_status_section_heights();
                        let repo_sidebar_collapsed_items =
                            this.sidebar_pane.read(cx).saved_sidebar_collapsed_items();
                        let repo_sidebar_pinned_branches =
                            this.sidebar_pane.read(cx).saved_sidebar_pinned_branches();
                        let font_preferences = crate::font_preferences::current(cx);

                        let settings = session::UiSettings {
                            window_width,
                            window_height,
                            sidebar_width: ui_scale::stored_design_units(
                                Some(this.ui_scale().design_units_from_pixels(this.sidebar_width)),
                            ),
                            details_width: ui_scale::stored_design_units(
                                Some(this.ui_scale().design_units_from_pixels(this.details_width)),
                            ),
                            sidebar_collapsed: Some(this.sidebar_collapsed),
                            repo_sidebar_collapsed_items: Some(repo_sidebar_collapsed_items),
                            repo_sidebar_pinned_branches: Some(repo_sidebar_pinned_branches),
                            theme_mode: Some(this.theme_mode.key().to_string()),
                            ui_scale_percent: Some(this.ui_scale_percent),
                            ui_font_family: Some(font_preferences.ui_font_family),
                            editor_font_family: Some(font_preferences.editor_font_family),
                            use_font_ligatures: Some(font_preferences.use_font_ligatures),
                            date_time_format: Some(this.date_time_format.key().to_string()),
                            timezone: Some(this.timezone.key()),
                            show_timezone: Some(this.show_timezone),
                            change_tracking_view: Some(this.change_tracking_view.key().to_string()),
                            // Owned by the repository picker, not this snapshot.
                            repo_picker_sort: None,
                            repo_picker_collapsed_sections: None,
                            diff_scroll_sync: Some(this.diff_scroll_sync.key().to_string()),
                            diff_content_mode: Some(this.diff_content_mode.key().to_string()),
                            diff_whitespace_mode: Some(
                                this.diff_whitespace_mode.key().to_string(),
                            ),
                            diff_view_mode: Some(this.diff_view_mode.key().to_string()),
                            annotate_enabled: Some(this.annotate_enabled),
                            diff_reveal_whitespace_chars: Some(
                                this.diff_reveal_whitespace_chars,
                            ),
                            diff_word_wrap: Some(this.diff_word_wrap),
                            diff_show_line_numbers: Some(this.diff_show_line_numbers),
                            // Auto-save is only ever changed from the settings
                            // window; the main window mirrors it to drive the
                            // editor, so None keeps the stored value.
                            auto_save_file_edits: None,
                            mergetool_auto_advance: Some(mergetool_auto_advance),
                            mergetool_collapse_unchanged: Some(mergetool_collapse_unchanged),
                            mergetool_output_scroll_sync: Some(mergetool_output_scroll_sync),
                            mergetool_show_line_numbers: Some(mergetool_show_line_numbers),
                            mergetool_view_three_way: Some(mergetool_view_three_way),
                            change_tracking_height,
                            untracked_height,
                            history_show_graph: Some(history_show_graph),
                            history_show_author: Some(history_show_author),
                            history_show_date: Some(history_show_date),
                            history_show_sha: Some(history_show_sha),
                            history_relative_dates: Some(history_relative_dates),
                            terminal_external_mode: None,
                            terminal_external_program: None,
                            terminal_external_args: None,
                            terminal_action_bar_target: None,
                            history_show_tags: Some(history_show_tags),
                            history_tag_fetch_mode: Some(if history_auto_fetch_tags_on_repo_activation
                            {
                                gitcomet_state::model::GitLogTagFetchMode::OnRepositoryActivation
                            } else {
                                gitcomet_state::model::GitLogTagFetchMode::Disabled
                            }),
                            default_history_mode: None,
                            commit_push_after_enabled: Some(this.commit_push_after_enabled),
                            default_tag_type: None,
                            git_executable_path: None,
                            external_code_editor: None,
                        };

                        Some(settings)
                    })
                    .ok()
                    .flatten();

                let Some(settings) = settings else {
                    return;
                };

                let _ = smol::unblock(move || session::persist_ui_settings(settings)).await;
            },
        )
        .detach();
    }

    pub(super) fn clamp_pane_widths_to_window(&mut self) {
        let total_w = self.last_window_size.width;
        if total_w.is_zero() {
            return;
        }

        let sidebar_handle_w = if self.sidebar_collapsed {
            px(0.0)
        } else {
            self.pane_resize_handle_width()
        };
        let details_handle_w = if self.details_collapsed {
            px(0.0)
        } else {
            self.pane_resize_handle_width()
        };
        let handles_w = sidebar_handle_w + details_handle_w;
        let main_min = self.main_min_width();
        let sidebar_min = self.sidebar_min_width();
        let details_min = self.details_min_width();
        let collapsed_w = self.pane_collapsed_width();

        if !self.sidebar_collapsed {
            let details_w = if self.details_collapsed {
                collapsed_w
            } else {
                self.details_width.max(details_min)
            };
            let max_sidebar = (total_w - details_w - main_min - handles_w).max(sidebar_min);
            self.set_sidebar_width_from_pixels(
                self.sidebar_width.max(sidebar_min).min(max_sidebar),
            );
        } else {
            self.set_sidebar_width_from_pixels(self.sidebar_width.max(sidebar_min));
        }

        if !self.details_collapsed {
            let sidebar_w = if self.sidebar_collapsed {
                collapsed_w
            } else {
                self.sidebar_width.max(sidebar_min)
            };
            let max_details = (total_w - sidebar_w - main_min - handles_w).max(details_min);
            self.set_details_width_from_pixels(
                self.details_width.max(details_min).min(max_details),
            );
        } else {
            self.set_details_width_from_pixels(self.details_width.max(details_min));
        }

        let sidebar_target = if self.sidebar_collapsed {
            collapsed_w
        } else {
            self.sidebar_width
        };
        let details_target = if self.details_collapsed {
            collapsed_w
        } else {
            self.details_width
        };

        if !self.sidebar_width_animating {
            self.sidebar_render_width = sidebar_target;
        } else {
            self.sidebar_render_width = self.sidebar_render_width.max(px(0.0)).min(total_w);
        }
        if !self.details_width_animating {
            self.details_render_width = details_target;
        } else {
            self.details_render_width = self.details_render_width.max(px(0.0)).min(total_w);
        }
    }
}
