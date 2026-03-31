use super::*;
use gitcomet_core::domain::FileConflictKind;
use gitcomet_core::services::ConflictSide;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct KeepDeleteConflictSpec {
    header_label: &'static str,
    description: &'static str,
    keep_label: &'static str,
    delete_label: &'static str,
    keep_side: ConflictSide,
    deleted_side_label: &'static str,
    surviving_side_label: &'static str,
}

fn keep_delete_conflict_spec(conflict_kind: FileConflictKind) -> KeepDeleteConflictSpec {
    match conflict_kind {
        FileConflictKind::DeletedByUs => KeepDeleteConflictSpec {
            header_label: "Modify / Delete conflict",
            description: "This file was modified on the remote branch but deleted on your local branch.",
            keep_label: "Keep File (theirs)",
            delete_label: "Accept Deletion (ours)",
            keep_side: ConflictSide::Theirs,
            deleted_side_label: "Ours",
            surviving_side_label: "Theirs",
        },
        FileConflictKind::DeletedByThem => KeepDeleteConflictSpec {
            header_label: "Modify / Delete conflict",
            description: "This file was modified on your local branch but deleted on the remote branch.",
            keep_label: "Keep File (ours)",
            delete_label: "Accept Deletion (theirs)",
            keep_side: ConflictSide::Ours,
            deleted_side_label: "Theirs",
            surviving_side_label: "Ours",
        },
        FileConflictKind::AddedByUs => KeepDeleteConflictSpec {
            header_label: "Add / Delete conflict",
            description: "This file was added on your local branch and deleted on the remote branch.",
            keep_label: "Keep File (ours)",
            delete_label: "Accept Deletion (theirs)",
            keep_side: ConflictSide::Ours,
            deleted_side_label: "Theirs",
            surviving_side_label: "Ours",
        },
        FileConflictKind::AddedByThem => KeepDeleteConflictSpec {
            header_label: "Add / Delete conflict",
            description: "This file was added on the remote branch and deleted on your local branch.",
            keep_label: "Keep File (theirs)",
            delete_label: "Accept Deletion (ours)",
            keep_side: ConflictSide::Theirs,
            deleted_side_label: "Ours",
            surviving_side_label: "Theirs",
        },
        // Shouldn't happen — only the four kinds above use this strategy.
        _ => KeepDeleteConflictSpec {
            header_label: "Conflict",
            description: "Unexpected conflict type.",
            keep_label: "Use Ours",
            delete_label: "Accept Deletion",
            keep_side: ConflictSide::Ours,
            deleted_side_label: "Theirs",
            surviving_side_label: "Ours",
        },
    }
}

fn conflict_side_has_payload(
    file: &gitcomet_state::model::ConflictFile,
    side: ConflictSide,
) -> bool {
    match side {
        ConflictSide::Ours => file.ours.is_some() || file.ours_bytes.is_some(),
        ConflictSide::Theirs => file.theirs.is_some() || file.theirs_bytes.is_some(),
    }
}

fn conflict_side_text(
    file: &gitcomet_state::model::ConflictFile,
    side: ConflictSide,
) -> Option<&str> {
    match side {
        ConflictSide::Ours => file.ours.as_deref(),
        ConflictSide::Theirs => file.theirs.as_deref(),
    }
}

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
        file: &gitcomet_state::model::ConflictFile,
        conflict_kind: FileConflictKind,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let spec = keep_delete_conflict_spec(conflict_kind);
        let editor_font_family = crate::font_preferences::current_editor_font_family(cx);

        let keep_available = conflict_side_has_payload(file, spec.keep_side);
        let surviving_text: Option<&str> = conflict_side_text(file, spec.keep_side);

        let surviving_lines: Vec<SharedString> = match surviving_text {
            Some(text) if !text.is_empty() => text
                .lines()
                .map(|l| SharedString::from(l.to_string()))
                .collect(),
            _ if keep_available => vec!["(empty file)".into()],
            _ => vec!["(not present in conflict stages)".into()],
        };
        let surviving_line_count = surviving_lines.len();
        let surviving_text: SharedString = surviving_lines.join("\n").into();

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
                components::Button::new("keep_delete_keep", spec.keep_label)
                    .style(components::ButtonStyle::Filled)
                    .disabled(!keep_available)
                    .on_click(theme, cx, move |this, _e, _w, _cx| {
                        this.store.dispatch(Msg::CheckoutConflictSide {
                            repo_id,
                            path: keep_path.clone(),
                            side: spec.keep_side,
                        });
                    }),
            )
            .child(
                components::Button::new("keep_delete_delete", spec.delete_label)
                    .style(components::ButtonStyle::Outlined)
                    .on_click(theme, cx, move |this, _e, _w, _cx| {
                        this.store.dispatch(Msg::AcceptConflictDeletion {
                            repo_id,
                            path: delete_path.clone(),
                        });
                    }),
            )
            .when(show_external_mergetool_actions(self.view_mode), |d| {
                d.child(div().w(px(1.0)).h(px(16.0)).bg(theme.colors.border))
                    .child(
                        components::Button::new("keep_delete_mergetool", "External Mergetool")
                            .style(components::ButtonStyle::Outlined)
                            .on_click(theme, cx, move |this, _e, _w, _cx| {
                                this.store.dispatch(Msg::LaunchMergetool {
                                    repo_id,
                                    path: mergetool_path.clone(),
                                });
                            }),
                    )
            });

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
                                            .child(spec.header_label),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(theme.colors.text_muted)
                                            .child(spec.description),
                                    ),
                            ),
                    )
                    .when(!keep_available, |d| {
                        d.child(
                            div()
                                .px_3()
                                .py_1()
                                .bg(theme.colors.surface_bg_elevated)
                                .border_b_1()
                                .border_color(theme.colors.border)
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.colors.warning)
                                        .child(
                                            "The keep side is unavailable in conflict stages; only deletion can be applied.",
                                        ),
                                ),
                        )
                    })
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
                                format!("Deleted side ({}):", spec.deleted_side_label),
                            ))
                            .child(
                                div()
                                    .mt_1()
                                    .text_sm()
                                    .font_family(editor_font_family.clone())
                                    .text_color(theme.colors.text_muted)
                                    .whitespace_nowrap()
                                    .child("(file deleted)"),
                            )
                            .child(div().mt_2().text_sm().text_color(theme.colors.text_muted).child(
                                format!(
                                    "Surviving side ({}) ({} line{}):",
                                    spec.surviving_side_label,
                                    surviving_line_count,
                                    if surviving_line_count == 1 { "" } else { "s" }
                                ),
                            ))
                            .child(
                                div()
                                    .mt_1()
                                    .text_sm()
                                    .font_family(editor_font_family)
                                    .text_color(theme.colors.text)
                                    .whitespace_nowrap()
                                    .child(surviving_text),
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

#[cfg(test)]
mod tests {
    use super::{conflict_side_has_payload, keep_delete_conflict_spec};
    use gitcomet_core::domain::FileConflictKind;
    use gitcomet_core::services::ConflictSide;
    use gitcomet_state::model::ConflictFile;
    use std::path::PathBuf;

    fn empty_conflict_file() -> ConflictFile {
        ConflictFile {
            path: PathBuf::from("a.txt").into(),
            base_bytes: None,
            ours_bytes: None,
            theirs_bytes: None,
            current_bytes: None,
            base: None,
            ours: None,
            theirs: None,
            current: None,
        }
    }

    #[test]
    fn keep_delete_spec_uses_add_delete_copy_for_added_variants() {
        let ours = keep_delete_conflict_spec(FileConflictKind::AddedByUs);
        assert_eq!(ours.header_label, "Add / Delete conflict");
        assert_eq!(ours.keep_side, ConflictSide::Ours);
        assert_eq!(ours.delete_label, "Accept Deletion (theirs)");

        let theirs = keep_delete_conflict_spec(FileConflictKind::AddedByThem);
        assert_eq!(theirs.header_label, "Add / Delete conflict");
        assert_eq!(theirs.keep_side, ConflictSide::Theirs);
        assert_eq!(theirs.delete_label, "Accept Deletion (ours)");
    }

    #[test]
    fn keep_delete_spec_uses_modify_delete_copy_for_deleted_variants() {
        let ours = keep_delete_conflict_spec(FileConflictKind::DeletedByUs);
        assert_eq!(ours.header_label, "Modify / Delete conflict");
        assert_eq!(ours.keep_side, ConflictSide::Theirs);

        let theirs = keep_delete_conflict_spec(FileConflictKind::DeletedByThem);
        assert_eq!(theirs.header_label, "Modify / Delete conflict");
        assert_eq!(theirs.keep_side, ConflictSide::Ours);
    }

    #[test]
    fn conflict_side_has_payload_detects_text_or_bytes() {
        let mut file = empty_conflict_file();
        assert!(!conflict_side_has_payload(&file, ConflictSide::Ours));
        assert!(!conflict_side_has_payload(&file, ConflictSide::Theirs));

        file.ours = Some("ours".into());
        assert!(conflict_side_has_payload(&file, ConflictSide::Ours));
        assert!(!conflict_side_has_payload(&file, ConflictSide::Theirs));

        file.theirs = None;
        file.theirs_bytes = Some(vec![0xff, 0x00].into());
        assert!(conflict_side_has_payload(&file, ConflictSide::Theirs));
    }
}
