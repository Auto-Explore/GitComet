use super::*;
use gitgpui_core::domain::FileConflictKind;
use gitgpui_core::services::ConflictSide;

impl MainPaneView {
    /// Render the keep/delete conflict resolver panel for modify/delete conflicts.
    ///
    /// Used for `DeletedByUs`, `DeletedByThem`, `AddedByUs`, `AddedByThem`.
    /// Shows the surviving side's content as a preview and offers explicit
    /// "Keep File" / "Accept Deletion" actions, plus mergetool fallback.
    pub(super) fn render_keep_delete_conflict_resolver(
        &mut self,
        theme: AppTheme,
        repo_id: RepoId,
        path: std::path::PathBuf,
        file: &gitgpui_state::model::ConflictFile,
        conflict_kind: FileConflictKind,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        // Determine which side has content and which was deleted, and the
        // human-readable labels for each action.
        let (description, keep_label, delete_label, keep_side): (
            &'static str,
            &'static str,
            &'static str,
            ConflictSide,
        ) = match conflict_kind {
            FileConflictKind::DeletedByUs => (
                "This file was modified on the remote branch but deleted on your local branch.",
                "Keep File (theirs)",
                "Accept Deletion (ours)",
                ConflictSide::Theirs,
            ),
            FileConflictKind::DeletedByThem => (
                "This file was modified on your local branch but deleted on the remote branch.",
                "Keep File (ours)",
                "Accept Deletion (theirs)",
                ConflictSide::Ours,
            ),
            FileConflictKind::AddedByUs => (
                "This file was added only on your local branch.",
                "Keep File (ours)",
                "Remove File",
                ConflictSide::Ours,
            ),
            FileConflictKind::AddedByThem => (
                "This file was added only on the remote branch.",
                "Keep File (theirs)",
                "Remove File",
                ConflictSide::Theirs,
            ),
            // Shouldn't happen — only the four kinds above use this strategy.
            _ => (
                "Unexpected conflict type.",
                "Use Ours",
                "Use Theirs",
                ConflictSide::Ours,
            ),
        };

        // Get the surviving content for preview.
        let surviving_text: Option<&str> = match keep_side {
            ConflictSide::Ours => file.ours.as_deref(),
            ConflictSide::Theirs => file.theirs.as_deref(),
        };

        let preview_lines: Vec<SharedString> = match surviving_text {
            Some(text) if !text.is_empty() => text
                .lines()
                .map(|l| SharedString::from(l.to_string()))
                .collect(),
            _ => vec!["(empty file)".into()],
        };
        let preview_line_count = preview_lines.len();
        let preview_text: SharedString = preview_lines.join("\n").into();

        let keep_path = path.clone();
        let delete_path = path.clone();
        let mergetool_path = path.clone();

        let title: SharedString =
            format!("Resolve conflict: {}", self.cached_path_display(&path)).into();

        let action_section = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                zed::Button::new("keep_delete_keep", keep_label)
                    .style(zed::ButtonStyle::Filled)
                    .on_click(theme, cx, move |this, _e, _w, _cx| {
                        this.store.dispatch(Msg::CheckoutConflictSide {
                            repo_id,
                            path: keep_path.clone(),
                            side: keep_side,
                        });
                    }),
            )
            .child(
                zed::Button::new("keep_delete_delete", delete_label)
                    .style(zed::ButtonStyle::Outlined)
                    .on_click(theme, cx, move |this, _e, _w, _cx| {
                        this.store.dispatch(Msg::AcceptConflictDeletion {
                            repo_id,
                            path: delete_path.clone(),
                        });
                    }),
            )
            .child(div().w(px(1.0)).h(px(16.0)).bg(theme.colors.border))
            .child(
                zed::Button::new("keep_delete_mergetool", "External Mergetool")
                    .style(zed::ButtonStyle::Outlined)
                    .on_click(theme, cx, move |this, _e, _w, _cx| {
                        this.store.dispatch(Msg::LaunchMergetool {
                            repo_id,
                            path: mergetool_path.clone(),
                        });
                    }),
            );

        div()
            .id("keep_delete_conflict_resolver_panel")
            .flex()
            .flex_col()
            .w_full()
            .h_full()
            .min_h(px(0.0))
            .overflow_hidden()
            .px_2()
            .py_2()
            .gap_2()
            // Header
            .child(
                div().flex().items_center().gap_2().child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.colors.text)
                        .child(title),
                ),
            )
            // Content panel
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .border_1()
                    .border_color(theme.colors.border)
                    .rounded(px(theme.radii.row))
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .bg(theme.colors.window_bg)
                    // Conflict description
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .px_3()
                            .py_2()
                            .bg(theme.colors.surface_bg_elevated)
                            .border_b_1()
                            .border_color(theme.colors.border)
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .items_center()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme.colors.warning)
                                            .child("Modify / Delete conflict"),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(theme.colors.text_muted)
                                            .child(description),
                                    ),
                            ),
                    )
                    // File preview
                    .child(
                        div()
                            .id("keep_delete_preview_scroll")
                            .flex_1()
                            .min_h(px(0.0))
                            .overflow_y_scroll()
                            .px_3()
                            .py_2()
                            .child(div().text_sm().text_color(theme.colors.text_muted).child(
                                format!(
                                    "File content ({} line{}):",
                                    preview_line_count,
                                    if preview_line_count == 1 { "" } else { "s" }
                                ),
                            ))
                            .child(
                                div()
                                    .mt_1()
                                    .text_sm()
                                    .font_family("monospace")
                                    .text_color(theme.colors.text)
                                    .whitespace_nowrap()
                                    .child(preview_text),
                            ),
                    )
                    // Action buttons
                    .child(
                        div()
                            .border_t_1()
                            .border_color(theme.colors.border)
                            .px_3()
                            .py_2()
                            .child(action_section),
                    ),
            )
            .into_any_element()
    }
}
