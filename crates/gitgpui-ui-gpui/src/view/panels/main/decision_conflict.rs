use super::*;
use gitgpui_core::services::ConflictSide;

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
        let has_base = file.base.is_some() || file.base_bytes.is_some();

        let accept_path = path.clone();
        let restore_path = path.clone();
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
            .when_some(file.base.clone().filter(|_| has_base), |d, base_text| {
                let p = restore_path.clone();
                d.child(
                    zed::Button::new("decision_restore_base", "Restore from Base")
                        .style(zed::ButtonStyle::Outlined)
                        .on_click(theme, cx, move |this, _e, _w, _cx| {
                            this.store.dispatch(Msg::SaveWorktreeFile {
                                repo_id,
                                path: p.clone(),
                                contents: base_text.clone(),
                                stage: true,
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
                    .when(has_base, |d| {
                        d.child(
                            div()
                                .text_xs()
                                .text_color(theme.colors.text_muted)
                                .child("A base version is available for restoration if needed."),
                        )
                    })
                    .child(action_section),
            )
            .into_any_element()
    }
}
