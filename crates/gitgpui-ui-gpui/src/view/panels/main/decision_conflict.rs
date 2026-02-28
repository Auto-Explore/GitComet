use super::*;
use gitgpui_core::services::ConflictSide;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct DecisionRestoreAvailability {
    has_base: bool,
    has_ours: bool,
    has_theirs: bool,
}

fn decision_restore_availability(
    file: &gitgpui_state::model::ConflictFile,
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
    /// the removal) or, if base content is available, restore from the base.
    pub(super) fn render_decision_conflict_resolver(
        &mut self,
        theme: AppTheme,
        repo_id: RepoId,
        path: std::path::PathBuf,
        file: &gitgpui_state::model::ConflictFile,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let restore = decision_restore_availability(file);
        let restore_summary = decision_restore_sources_summary(restore);

        let accept_path = path.clone();
        let restore_path = path.clone();
        let restore_ours_path = path.clone();
        let restore_theirs_path = path.clone();
        let mergetool_path = path.clone();

        let title: SharedString =
            format!("Resolve conflict: {}", self.cached_path_display(&path)).into();

        let action_section = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                zed::Button::new("decision_accept_delete", "Accept Deletion")
                    .style(zed::ButtonStyle::Filled)
                    .on_click(theme, cx, move |this, _e, _w, _cx| {
                        // Accepting the deletion: checkout --ours resolves the
                        // conflict by accepting the deletion (both sides agree).
                        this.store.dispatch(Msg::CheckoutConflictSide {
                            repo_id,
                            path: accept_path.clone(),
                            side: ConflictSide::Ours,
                        });
                    }),
            )
            .when(restore.has_ours, |d| {
                let p = restore_ours_path.clone();
                d.child(
                    zed::Button::new("decision_restore_ours", "Restore Ours")
                        .style(zed::ButtonStyle::Outlined)
                        .on_click(theme, cx, move |this, _e, _w, _cx| {
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
                d.child(
                    zed::Button::new("decision_restore_theirs", "Restore Theirs")
                        .style(zed::ButtonStyle::Outlined)
                        .on_click(theme, cx, move |this, _e, _w, _cx| {
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
                d.child(
                    zed::Button::new("decision_restore_base", "Restore from Base")
                        .style(zed::ButtonStyle::Outlined)
                        .on_click(theme, cx, move |this, _e, _w, _cx| {
                            this.store.dispatch(Msg::CheckoutConflictBase {
                                repo_id,
                                path: p.clone(),
                            });
                        }),
                )
            })
            .child(div().w(px(1.0)).h(px(16.0)).bg(theme.colors.border))
            .child(
                zed::Button::new("decision_mergetool", "External Mergetool")
                    .style(zed::ButtonStyle::Outlined)
                    .on_click(theme, cx, move |this, _e, _w, _cx| {
                        this.store.dispatch(Msg::LaunchMergetool {
                            repo_id,
                            path: mergetool_path.clone(),
                        });
                    }),
            );

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
                    .items_center()
                    .justify_center()
                    .gap_4()
                    .bg(theme.colors.window_bg)
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.colors.warning)
                            .child("Both sides deleted this file"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.colors.text_muted)
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
                                .text_color(theme.colors.text_muted)
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
        decision_restore_availability, decision_restore_sources_summary, DecisionRestoreAvailability,
    };
    use gitgpui_state::model::ConflictFile;
    use std::path::PathBuf;

    fn empty_conflict_file() -> ConflictFile {
        ConflictFile {
            path: PathBuf::from("a.txt"),
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
        file.ours_bytes = Some(vec![0xff, 0x00]);

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
