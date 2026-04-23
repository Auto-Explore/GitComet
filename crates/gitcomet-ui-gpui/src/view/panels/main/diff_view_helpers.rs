use super::*;

impl MainPaneView {
    pub(super) fn diff_panel_title(&self, theme: AppTheme, cx: &gpui::Context<Self>) -> AnyElement {
        self.active_repo()
            .and_then(|r| r.diff_state.diff_target.as_ref())
            .map(|t| {
                let (icon, color, text): (Option<&'static str>, gpui::Rgba, SharedString) = match t
                {
                    DiffTarget::WorkingTree { path, area } => {
                        let kind = self.active_repo().and_then(|repo| {
                            repo.status_entry_for_path(*area, path.as_path())
                                .map(|entry| entry.kind)
                        });

                        let (icon, color) = match kind.unwrap_or(FileStatusKind::Modified) {
                            FileStatusKind::Untracked | FileStatusKind::Added => {
                                ("icons/plus.svg", theme.colors.success)
                            }
                            FileStatusKind::Modified => ("icons/pencil.svg", theme.colors.warning),
                            FileStatusKind::Deleted => ("icons/minus.svg", theme.colors.danger),
                            FileStatusKind::Renamed => ("icons/swap.svg", theme.colors.accent),
                            FileStatusKind::Conflicted => {
                                ("icons/warning.svg", theme.colors.danger)
                            }
                        };
                        (Some(icon), color, self.cached_path_display(path))
                    }
                    DiffTarget::Commit { commit_id: _, path } => match path {
                        Some(path) => (
                            Some("icons/pencil.svg"),
                            theme.colors.text_muted,
                            self.cached_path_display(path),
                        ),
                        None => (
                            Some("icons/pencil.svg"),
                            theme.colors.text_muted,
                            "Full diff".into(),
                        ),
                    },
                };

                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(
                        div()
                            .w(px(16.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .when_some(icon, |this, icon| {
                                this.child(svg_icon(icon, color, px(14.0)))
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .child(
                                components::TruncatedText::new(text)
                                    .profile(components::TextTruncationProfile::Path)
                                    .tooltip_host(self.tooltip_host.clone())
                                    .tooltip_mode(
                                        components::TruncatedTextTooltipMode::FullTextIfTruncated,
                                    )
                                    .render(cx),
                            ),
                    )
                    .into_any_element()
            })
            .unwrap_or_else(|| {
                div()
                    .text_sm()
                    .font_weight(FontWeight::BOLD)
                    .child("Select a file to view diff")
                    .into_any_element()
            })
    }

    pub(super) fn diff_nav_hotkey_hint(theme: AppTheme, label: &'static str) -> gpui::Div {
        div()
            .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
            .text_xs()
            .text_color(theme.colors.text_muted)
            .child(label)
    }

    pub(super) fn diff_prev_next_file_buttons(
        &self,
        repo_id: Option<RepoId>,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> (Option<AnyElement>, Option<AnyElement>) {
        let buttons = (|| {
            let repo_id = repo_id?;
            let repo = self.active_repo()?;
            let change_tracking_view = self.active_change_tracking_view(cx);

            let diff_target = repo.diff_state.diff_target.as_ref()?;
            let prev = status_nav::adjacent_diff_file_target_for_repo(
                repo,
                diff_target,
                change_tracking_view,
                -1,
            );
            let next = status_nav::adjacent_diff_file_target_for_repo(
                repo,
                diff_target,
                change_tracking_view,
                1,
            );

            let prev_disabled = prev.is_none();
            let next_disabled = next.is_none();

            let prev_tooltip: SharedString = "Previous file (F1)".into();
            let next_tooltip: SharedString = "Next file (F4)".into();

            let prev_btn = components::Button::new("diff_prev_file", "Prev file")
                .separated_end_slot(Self::diff_nav_hotkey_hint(theme, "F1"))
                .style(components::ButtonStyle::Outlined)
                .disabled(prev_disabled)
                .on_click(theme, cx, move |this, _e, window, cx| {
                    if this.try_select_adjacent_diff_file(repo_id, -1, window, cx) {
                        cx.notify();
                    }
                })
                .on_hover(cx.listener(move |this, hovering: &bool, _w, cx| {
                    let mut changed = false;
                    if *hovering {
                        changed |= this.set_tooltip_text_if_changed(Some(prev_tooltip.clone()), cx);
                    } else {
                        changed |= this.clear_tooltip_if_matches(&prev_tooltip, cx);
                    }
                    if changed {
                        cx.notify();
                    }
                }))
                .into_any_element();

            let next_btn = components::Button::new("diff_next_file", "Next file")
                .separated_end_slot(Self::diff_nav_hotkey_hint(theme, "F4"))
                .style(components::ButtonStyle::Outlined)
                .disabled(next_disabled)
                .on_click(theme, cx, move |this, _e, window, cx| {
                    if this.try_select_adjacent_diff_file(repo_id, 1, window, cx) {
                        cx.notify();
                    }
                })
                .on_hover(cx.listener(move |this, hovering: &bool, _w, cx| {
                    let mut changed = false;
                    if *hovering {
                        changed |= this.set_tooltip_text_if_changed(Some(next_tooltip.clone()), cx);
                    } else {
                        changed |= this.clear_tooltip_if_matches(&next_tooltip, cx);
                    }
                    if changed {
                        cx.notify();
                    }
                }))
                .into_any_element();

            Some((prev_btn, next_btn))
        })();

        buttons
            .map(|(prev, next)| (Some(prev), Some(next)))
            .unwrap_or((None, None))
    }
}
