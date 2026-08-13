use super::*;

impl MainPaneView {
    pub(super) fn diff_panel_title(&self, theme: AppTheme, cx: &gpui::Context<Self>) -> AnyElement {
        self.rendered_diff_target()
            .map(|t| {
                let (icon, color, text): (Option<&'static str>, gpui::Rgba, SharedString) = match t
                {
                    DiffTarget::WorkingTree { path, area } => {
                        let kind = if self.is_inline_submodule_diff_active() {
                            self.selected_inline_submodule_diff_entry()
                                .map(|entry| entry.kind)
                        } else {
                            self.active_repo().and_then(|repo| {
                                repo.status_entry_for_path(*area, path.as_path())
                                    .map(|entry| entry.kind)
                            })
                        };

                        let (icon, color) = match kind.unwrap_or(FileStatusKind::Modified) {
                            FileStatusKind::Untracked | FileStatusKind::Added => {
                                ("icons/plus.svg", theme.colors.status.success.foreground)
                            }
                            FileStatusKind::Modified => {
                                ("icons/pencil.svg", theme.colors.status.warning.foreground)
                            }
                            FileStatusKind::Deleted => {
                                ("icons/minus.svg", theme.colors.status.danger.foreground)
                            }
                            FileStatusKind::Renamed => {
                                ("icons/swap.svg", theme.colors.accent.foreground)
                            }
                            FileStatusKind::Conflicted => {
                                ("icons/warning.svg", theme.colors.status.danger.foreground)
                            }
                        };
                        (Some(icon), color, self.cached_path_display(path))
                    }
                    DiffTarget::Commit { commit_id: _, path } => match path {
                        Some(path) => (
                            Some("icons/pencil.svg"),
                            theme.colors.foreground.secondary,
                            self.cached_path_display(path),
                        ),
                        None => (
                            Some("icons/pencil.svg"),
                            theme.colors.foreground.secondary,
                            "Full diff".into(),
                        ),
                    },
                    DiffTarget::CommitRange {
                        from_commit_id: _,
                        to_commit_id: _,
                        path,
                    } => match path {
                        Some(path) => (
                            Some("icons/swap.svg"),
                            theme.colors.accent.foreground,
                            self.cached_path_display(path),
                        ),
                        None => (
                            Some("icons/swap.svg"),
                            theme.colors.accent.foreground,
                            "Commit range".into(),
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
                                components::TruncatedText::path(text)
                                    .id(("diff_title_path", 0usize))
                                    .full_text_tooltip(self.tooltip_host.clone())
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

    /// Revision controls shown next to the file path in the file content
    /// viewer: back/forward through the cross-file viewer history, plus a
    /// clickable commit-SHA badge that opens the file-history menu. Returns
    /// `None` outside the file content viewer (diff and merge views).
    pub(super) fn diff_viewer_nav_cluster(
        &self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Option<AnyElement> {
        if !self.is_file_preview_active() {
            return None;
        }
        let repo = self.active_repo()?;
        let repo_id = repo.id;
        let can_back = repo.view_history.can_back();
        let can_forward = repo.view_history.can_forward();
        let ui_scale_percent = crate::ui_scale::UiScale::current(cx).percent();

        let (badge_label, path): (SharedString, std::path::PathBuf) =
            match self.rendered_diff_target()? {
                DiffTarget::Commit {
                    commit_id,
                    path: Some(path),
                } => (
                    commit_id
                        .as_ref()
                        .chars()
                        .take(8)
                        .collect::<String>()
                        .into(),
                    path.clone(),
                ),
                DiffTarget::WorkingTree { path, .. } => ("Working tree".into(), path.clone()),
                // Range diffs and full-tree commits are not file content views.
                _ => return None,
            };

        // Monospace label so the badge keeps a constant width as the SHA changes.
        let badge = div()
            .id("viewer_revision_badge")
            .flex()
            .items_center()
            .gap_1()
            .px_1()
            .h(components::control_height(ui_scale_percent))
            .rounded(px(theme.radii.row))
            .border_1()
            .border_color(theme.colors.stroke.default)
            .cursor(CursorStyle::PointingHand)
            .hover(move |s| s.bg(with_alpha(theme.colors.interaction.hover_background, 0.55)))
            .active(move |s| s.bg(theme.colors.interaction.pressed_background))
            .child(svg_icon(
                "icons/history.svg",
                theme.colors.foreground.secondary,
                px(12.0),
            ))
            .child(
                div()
                    .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                    .text_xs()
                    .whitespace_nowrap()
                    .child(badge_label),
            )
            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                this.open_popover_at(
                    PopoverKind::FileHistory {
                        repo_id,
                        path: path.clone(),
                    },
                    e.position(),
                    window,
                    cx,
                );
            }))
            .gitcomet_tooltip(theme, "Show file history".into());

        let back_btn = components::Button::new("viewer_nav_back", "")
            .start_slot(svg_icon(
                "icons/arrow_left.svg",
                theme.colors.foreground.primary,
                px(14.0),
            ))
            .style(components::ButtonStyle::Outlined)
            .disabled(!can_back)
            .on_click(theme, cx, move |this, _e, _w, cx| {
                this.store.dispatch(Msg::ViewerNavBack { repo_id });
                cx.notify();
            })
            .gitcomet_tooltip(theme, "Back to previous file version".into());

        let forward_btn = components::Button::new("viewer_nav_forward", "")
            .start_slot(svg_icon(
                "icons/arrow_right.svg",
                theme.colors.foreground.primary,
                px(14.0),
            ))
            .style(components::ButtonStyle::Outlined)
            .disabled(!can_forward)
            .on_click(theme, cx, move |this, _e, _w, cx| {
                this.store.dispatch(Msg::ViewerNavForward { repo_id });
                cx.notify();
            })
            .gitcomet_tooltip(theme, "Forward to next file version".into());

        // Badge first (immediately next to the path), then back/forward.
        Some(
            div()
                .flex()
                .items_center()
                .gap_1()
                .flex_none()
                .child(badge)
                .child(back_btn)
                .child(forward_btn)
                .into_any_element(),
        )
    }

    pub(super) fn diff_nav_hotkey_hint(theme: AppTheme, label: &'static str) -> gpui::Div {
        div()
            .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
            .text_xs()
            .text_color(theme.colors.foreground.secondary)
            .child(label)
    }

    pub(in crate::view) fn collapsed_diff_total_file_stat(&self) -> Option<(usize, usize)> {
        let (added, removed) = self.diff_file_stats.iter().filter_map(|stat| *stat).fold(
            (0usize, 0usize),
            |(added, removed), (next_added, next_removed)| {
                (
                    added.saturating_add(next_added),
                    removed.saturating_add(next_removed),
                )
            },
        );

        (added > 0 || removed > 0).then_some((added, removed))
    }

    pub(super) fn split_column_header_label(
        label: &'static str,
        count: Option<usize>,
        prefix: char,
        color: gpui::Rgba,
    ) -> AnyElement {
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .min_w(px(0.0))
            .child(
                div()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(label),
            )
            .when(count.is_some_and(|count| count > 0), |this| {
                let count = count.unwrap_or_default();
                let debug_selector = match prefix {
                    '-' => "diff_split_header_removed_stat",
                    '+' => "diff_split_header_added_stat",
                    _ => "diff_split_header_stat",
                };
                this.child(
                    div()
                        .debug_selector(move || debug_selector.to_string())
                        .flex_none()
                        .text_color(color)
                        .child(format!("{prefix}{count}")),
                )
            })
            .into_any_element()
    }

    /// The Previous/Next file buttons (F1 / F4).
    ///
    /// A side is `None` — not merely disabled — when there is no file to step
    /// to. A disabled arrow reads as "there is more here, but not right now",
    /// which is wrong for the two states that produce it: a file opened from the
    /// explorer has no navigation list at all, and the ends of a list have no
    /// neighbour on that side. Both showed a pair of permanently dead arrows.
    pub(super) fn diff_prev_next_file_buttons(
        &self,
        repo_id: Option<RepoId>,
        borderless: bool,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> (Option<AnyElement>, Option<AnyElement>) {
        let Some(repo_id) = repo_id else {
            return (None, None);
        };

        let (has_prev, has_next) = if let Some(inline) = self.active_inline_submodule_diff() {
            (
                inline.selected_ix > 0,
                inline.selected_ix + 1 < inline.entries.len(),
            )
        } else {
            let Some(repo) = self.active_repo() else {
                return (None, None);
            };
            let change_tracking_view = self.active_change_tracking_view(cx);
            let Some(diff_target) = repo.diff_state.diff_target.as_ref() else {
                return (None, None);
            };
            (
                status_nav::adjacent_diff_file_target_for_repo(
                    repo,
                    diff_target,
                    change_tracking_view,
                    -1,
                )
                .is_some(),
                status_nav::adjacent_diff_file_target_for_repo(
                    repo,
                    diff_target,
                    change_tracking_view,
                    1,
                )
                .is_some(),
            )
        };

        let button = |id: &'static str,
                      icon: &'static str,
                      tooltip: &'static str,
                      delta: i8,
                      cx: &mut gpui::Context<Self>| {
            let btn = components::Button::new(id, "")
                .start_slot(svg_icon(icon, theme.colors.foreground.primary, px(14.0)))
                .style(components::ButtonStyle::Outlined);
            let btn = if borderless { btn.borderless() } else { btn };
            btn.on_click(theme, cx, move |this, _e, window, cx| {
                if this.try_select_adjacent_diff_file(repo_id, delta, window, cx) {
                    cx.notify();
                }
            })
            .gitcomet_tooltip(theme, SharedString::from(tooltip))
            .into_any_element()
        };

        (
            has_prev.then(|| {
                button(
                    "diff_prev_file",
                    "icons/arrow_left.svg",
                    "Previous file (F1)",
                    -1,
                    cx,
                )
            }),
            has_next.then(|| {
                button(
                    "diff_next_file",
                    "icons/arrow_right.svg",
                    "Next file (F4)",
                    1,
                    cx,
                )
            }),
        )
    }
}
