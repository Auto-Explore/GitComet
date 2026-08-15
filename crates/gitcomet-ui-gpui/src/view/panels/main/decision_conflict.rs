use super::*;
use gitcomet_core::services::ConflictSide;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct DecisionRestoreAvailability {
    has_base: bool,
    has_ours: bool,
    has_theirs: bool,
}

fn decision_restore_availability(
    file: &gitcomet_state::model::ConflictFile,
) -> DecisionRestoreAvailability {
    DecisionRestoreAvailability {
        has_base: file.base.is_some() || file.base_bytes.is_some(),
        has_ours: file.ours.is_some() || file.ours_bytes.is_some(),
        has_theirs: file.theirs.is_some() || file.theirs_bytes.is_some(),
    }
}

fn decision_restore_sources_summary(availability: DecisionRestoreAvailability) -> Option<String> {
    let mut sources = Vec::new();
    if availability.has_base {
        sources.push("base");
    }
    if availability.has_ours {
        sources.push("ours");
    }
    if availability.has_theirs {
        sources.push("theirs");
    }
    if sources.is_empty() {
        None
    } else {
        Some(format!(
            "Restore sources available: {}.",
            sources.join(", ")
        ))
    }
}

impl MainPaneView {
    /// Render the decision-only conflict resolver for `BothDeleted` conflicts.
    ///
    /// Both sides deleted the file. The user can accept the deletion (stage
    /// the removal) or restore from any available staged source.
    pub(super) fn render_decision_conflict_resolver(
        &mut self,
        theme: AppTheme,
        repo_id: RepoId,
        path: std::path::PathBuf,
        file: &gitcomet_state::model::ConflictFile,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let restore = decision_restore_availability(file);
        let restore_summary = decision_restore_sources_summary(restore);

        let accept_path = path.clone();
        let restore_path = path.clone();
        let restore_ours_path = path.clone();
        let restore_theirs_path = path.clone();
        let mergetool_path = path.clone();
        let focused_mergetool = self.view_mode == GitCometViewMode::FocusedMergetool;
        let base_bytes = conflict_side_output_bytes(file, ThreeWayColumn::Base);
        let ours_bytes = conflict_side_output_bytes(file, ThreeWayColumn::Ours);
        let theirs_bytes = conflict_side_output_bytes(file, ThreeWayColumn::Theirs);

        let title: SharedString =
            format!("Resolve conflict: {}", self.cached_path_display(&path)).into();

        let action_section = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                components::Button::new(
                    "decision_accept_delete",
                    if focused_mergetool {
                        "Accept Deletion & close"
                    } else {
                        "Accept Deletion"
                    },
                )
                .style(components::ButtonStyle::Filled)
                .on_click(theme, cx, move |this, _e, _w, cx| {
                    if focused_mergetool {
                        this.focused_mergetool_delete_and_exit(repo_id, &accept_path, cx);
                        return;
                    }
                    this.store.dispatch(Msg::AcceptConflictDeletion {
                        repo_id,
                        path: accept_path.clone(),
                    });
                }),
            )
            .when(restore.has_ours, |d| {
                let p = restore_ours_path.clone();
                let bytes = ours_bytes.clone();
                d.child(
                    components::Button::new(
                        "decision_restore_ours",
                        if focused_mergetool {
                            "Restore Ours & close"
                        } else {
                            "Restore Ours"
                        },
                    )
                    .style(components::ButtonStyle::Outlined)
                    .on_click(theme, cx, move |this, _e, _w, cx| {
                        if focused_mergetool {
                            if let Some(bytes) = bytes.as_deref() {
                                this.focused_mergetool_write_side_and_exit(repo_id, &p, bytes, cx);
                            }
                            return;
                        }
                        this.store.dispatch(Msg::CheckoutConflictSide {
                            repo_id,
                            path: p.clone(),
                            side: ConflictSide::Ours,
                        });
                    }),
                )
            })
            .when(restore.has_theirs, |d| {
                let p = restore_theirs_path.clone();
                let bytes = theirs_bytes.clone();
                d.child(
                    components::Button::new(
                        "decision_restore_theirs",
                        if focused_mergetool {
                            "Restore Theirs & close"
                        } else {
                            "Restore Theirs"
                        },
                    )
                    .style(components::ButtonStyle::Outlined)
                    .on_click(theme, cx, move |this, _e, _w, cx| {
                        if focused_mergetool {
                            if let Some(bytes) = bytes.as_deref() {
                                this.focused_mergetool_write_side_and_exit(repo_id, &p, bytes, cx);
                            }
                            return;
                        }
                        this.store.dispatch(Msg::CheckoutConflictSide {
                            repo_id,
                            path: p.clone(),
                            side: ConflictSide::Theirs,
                        });
                    }),
                )
            })
            .when(restore.has_base, |d| {
                let p = restore_path.clone();
                let bytes = base_bytes.clone();
                d.child(
                    components::Button::new(
                        "decision_restore_base",
                        if focused_mergetool {
                            "Restore Base & close"
                        } else {
                            "Restore from Base"
                        },
                    )
                    .style(components::ButtonStyle::Outlined)
                    .on_click(theme, cx, move |this, _e, _w, cx| {
                        if focused_mergetool {
                            if let Some(bytes) = bytes.as_deref() {
                                this.focused_mergetool_write_side_and_exit(repo_id, &p, bytes, cx);
                            }
                            return;
                        }
                        this.store.dispatch(Msg::CheckoutConflictBase {
                            repo_id,
                            path: p.clone(),
                        });
                    }),
                )
            })
            .when(show_external_mergetool_actions(self.view_mode), |d| {
                d.child(div().w(px(1.0)).h(px(16.0)).bg(theme.colors.stroke.default))
                    .child(
                        components::Button::new("decision_mergetool", "External Mergetool")
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
            .id("decision_conflict_resolver_panel")
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
                        .text_color(theme.colors.foreground.primary)
                        .child(title),
                ),
            )
            // Content panel
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .border_1()
                    .border_color(theme.colors.stroke.default)
                    .rounded(px(theme.radii.row))
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_4()
                    .bg(theme.colors.surface.canvas)
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.colors.status.warning.foreground)
                            .child("Both sides deleted this file"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.colors.foreground.secondary)
                            .text_center()
                            .child(
                                "This file was deleted on both the local and remote branches. \
                                 Accept the deletion to resolve the conflict.",
                            ),
                    )
                    .when_some(restore_summary, |d, summary| {
                        d.child(
                            div()
                                .text_xs()
                                .text_color(theme.colors.foreground.secondary)
                                .child(summary),
                        )
                    })
                    .child(action_section),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DecisionRestoreAvailability, decision_restore_availability,
        decision_restore_sources_summary,
    };
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
    fn decision_restore_availability_detects_text_and_bytes_sources() {
        let mut file = empty_conflict_file();
        file.base = Some("base".into());
        file.ours_bytes = Some(vec![0xff, 0x00].into());

        assert_eq!(
            decision_restore_availability(&file),
            DecisionRestoreAvailability {
                has_base: true,
                has_ours: true,
                has_theirs: false
            }
        );
    }

    #[test]
    fn decision_restore_sources_summary_lists_sources_in_stable_order() {
        assert_eq!(
            decision_restore_sources_summary(DecisionRestoreAvailability {
                has_base: true,
                has_ours: true,
                has_theirs: true
            })
            .as_deref(),
            Some("Restore sources available: base, ours, theirs.")
        );
        assert_eq!(
            decision_restore_sources_summary(DecisionRestoreAvailability {
                has_base: false,
                has_ours: true,
                has_theirs: false
            })
            .as_deref(),
            Some("Restore sources available: ours.")
        );
        assert_eq!(
            decision_restore_sources_summary(DecisionRestoreAvailability::default()),
            None
        );
    }
}
