use super::*;
use crate::kit::interaction::{self as controls, ControlInteractionExt as _};

impl MainPaneView {
    /// The worktree chip for the diff currently on screen, when that diff comes
    /// from a linked worktree rather than this tab. `None` for everything else,
    /// including submodule diffs, which the file list already labels.
    fn foreign_worktree_diff_chip(
        &self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Option<AnyElement> {
        let inline = self.active_inline_submodule_diff()?;
        let gitcomet_state::model::ForeignDiffOrigin::Worktree { branch, detached } =
            &inline.origin
        else {
            return None;
        };
        let label = crate::view::rows::sidebar::worktree_origin_label(
            branch.as_deref(),
            *detached,
            &inline.submodule_repo_path,
        );

        let open_path = inline.submodule_repo_path.clone();
        // Scaled like the other two chips (the details pane's and the history
        // row's): an unscaled chip stops matching the title row it sits in.
        let ui_scale = crate::ui_scale::UiScale::current(cx);
        Some(
            crate::view::rows::sidebar::worktree_origin_chip(
                "diff_title_worktree_origin",
                theme,
                label,
                ui_scale.px(10.0),
                crate::view::rows::sidebar::worktree_badge_height(ui_scale),
                ui_scale.px(220.0),
                ui_scale.px(6.0),
            )
            .gitcomet_tooltip(
                theme,
                format!(
                    "Open this worktree in a tab\n{}",
                    inline.submodule_repo_path.display()
                )
                .into(),
            )
            .on_activate(
                false,
                controls::ControlActivation::Nested,
                cx.listener(move |_this, e: &ClickEvent, window, cx| {
                    if !e.standard_click() {
                        return;
                    }
                    cx.stop_propagation();
                    crate::app::open_repository_from_view(
                        cx,
                        window.window_handle().window_id(),
                        open_path.clone(),
                    );
                    cx.notify();
                }),
            )
            .into_any_element(),
        )
    }

    /// How the shown file changed, for the header's icon: from the status
    /// entry, the inline diff's entry, or the commit's or comparison's file
    /// list. `None` when no list knows the file.
    pub(in crate::view) fn rendered_file_kind(&self) -> Option<FileStatusKind> {
        let repo = self.active_repo()?;
        match self.rendered_diff_target()? {
            DiffTarget::WorkingTree { path, area, .. } => {
                if self.is_inline_submodule_diff_active() {
                    self.selected_inline_submodule_diff_entry()
                        .map(|entry| entry.kind)
                } else {
                    repo.status_entry_for_path(*area, path.as_path())
                        .map(|entry| entry.kind)
                }
            }
            DiffTarget::Commit {
                commit_id, path, ..
            } => match &repo.history_state.commit_details {
                Loadable::Ready(details) if &details.id == commit_id => details
                    .files
                    .iter()
                    .find(|file| &file.path == path)
                    .map(|file| file.kind),
                _ => None,
            },
            DiffTarget::CommitRange {
                path: Some(path), ..
            } => match &repo.history_state.range_files {
                Loadable::Ready(files) => files
                    .iter()
                    .find(|file| &file.path == path)
                    .map(|file| file.kind),
                _ => None,
            },
            DiffTarget::CommitRange { path: None, .. } => None,
        }
    }

    pub(super) fn diff_panel_title(
        &self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let ui_scale = crate::ui_scale::UiScale::current(cx);
        let header_bg = if self.is_file_preview_active() && self.historical_browse_content_active()
        {
            crate::theme::historical_header_bg(theme, crate::theme::content_header_bg(theme))
        } else {
            crate::theme::content_header_bg(theme)
        };
        self.rendered_diff_target()
            .map(|target| {
                // The file lists' icon: the file's type, its change badged on
                // the corner. A whole comparison has no single file.
                let path = match target {
                    DiffTarget::WorkingTree { path, .. } | DiffTarget::Commit { path, .. } => {
                        Some(path)
                    }
                    DiffTarget::CommitRange { path, .. } => path.as_ref(),
                };
                let (icon, text): (AnyElement, SharedString) = match path {
                    Some(path) => {
                        let kind = self
                            .rendered_file_kind()
                            .unwrap_or(FileStatusKind::Modified);
                        let (icon, color) = crate::view::rows::file_row_icon(path, kind, &theme);
                        let badge = crate::view::rows::file_row_kind_badge(kind, &theme);
                        (
                            crate::view::rows::file_row_icon_slot(
                                icon,
                                color,
                                badge,
                                crate::view::rows::FileRowBadgeDisc::still(header_bg),
                                14.0,
                                16.0,
                                ui_scale.percent(),
                            )
                            .debug_selector(|| "diff_title_icon".to_string())
                            .into_any_element(),
                            self.cached_path_display(path),
                        )
                    }
                    None => (
                        div()
                            .w(ui_scale.px(16.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(svg_icon(
                                "icons/swap.svg",
                                theme.colors.accent.foreground,
                                ui_scale.px(14.0),
                            ))
                            .into_any_element(),
                        "Commit range".into(),
                    ),
                };
                // The folder in grey, the name in full contrast, and the name
                // kept in view when the path is cut short.
                let name_start = text.rfind(['/', '\\']).map_or(0, |ix| ix + 1);
                let folder = gpui::HighlightStyle {
                    color: Some(palette::IntoColor::into_color(
                        theme.colors.foreground.secondary,
                    )),
                    font_weight: Some(FontWeight::NORMAL),
                    ..gpui::HighlightStyle::default()
                };
                let name =
                    (path.is_some() && name_start < text.len()).then(|| name_start..text.len());

                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(icon)
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_size(theme.ui_text(14.0))
                            .font_weight(FontWeight::BOLD)
                            .child(
                                // Given its colour: the text measures itself
                                // outside this div's style, and in a hosted
                                // pane the colour it found there was black.
                                components::TruncatedText::path(text, theme.ui_text(14.0))
                                    .text_color(theme.colors.foreground.primary)
                                    .highlights((name_start > 0).then_some((0..name_start, folder)))
                                    .focus_range(name)
                                    .id(("diff_title_path", 0usize))
                                    .full_text_tooltip(self.tooltip_host.clone())
                                    .render(cx),
                            ),
                    )
                    // These contents belong to another checkout, and the path
                    // alone gives no hint of that.
                    .children(self.foreign_worktree_diff_chip(theme, cx))
                    .into_any_element()
            })
            .unwrap_or_else(|| {
                div()
                    .text_size(theme.ui_text(14.0))
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
        let can_back = repo.navigation.view_history.can_back();
        let can_forward = repo.navigation.view_history.can_forward();
        let ui_scale_percent = crate::ui_scale::UiScale::current(cx).percent();
        let scaled_px = crate::ui_scale::scaler(ui_scale_percent);

        let (badge_label, path): (SharedString, std::path::PathBuf) =
            match self.rendered_diff_target()? {
                DiffTarget::Commit {
                    commit_id, path, ..
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
                // Range diffs are not file content views.
                _ => return None,
            };

        let history_invoker: SharedString = "viewer_revision_badge".into();
        let history_open = self.active_context_menu_invoker.as_ref() == Some(&history_invoker);
        // Monospace label so the badge keeps a constant width as the SHA changes.
        let badge = div()
            .id("viewer_revision_badge")
            .flex()
            .items_center()
            .gap_1()
            .px_1()
            .h(components::control_height(
                ui_scale::UiScale::from_percent(ui_scale_percent).with_appearance(theme.metrics),
            ))
            .rounded(px(theme.radii.row))
            .border_1()
            .border_color(theme.colors.stroke.default)
            .cursor(CursorStyle::PointingHand)
            .control_interaction(
                controls::InteractionStyle::header(theme),
                controls::InteractionState::default().open(history_open),
            )
            .child(svg_icon(
                "icons/history.svg",
                theme.colors.foreground.secondary,
                scaled_px(12.0),
            ))
            .child(
                div()
                    .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                    .text_size(theme.ui_text(12.0))
                    .whitespace_nowrap()
                    .child(badge_label),
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(move |this, e: &ClickEvent, window, cx| {
                    this.open_popover_at(
                        (PopoverKind::FileHistory {
                            repo_id,
                            path: path.clone(),
                        })
                        .invoked_by(history_invoker.clone()),
                        e.position(),
                        window,
                        cx,
                    );
                }),
            )
            .gitcomet_tooltip(theme, "Show file history".into());

        let back_btn = components::Button::new("viewer_nav_back", "")
            .start_slot(svg_icon(
                "icons/arrow_left.svg",
                theme.colors.foreground.primary,
                scaled_px(14.0),
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
                scaled_px(14.0),
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
            .text_size(theme.ui_text(12.0))
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
}
