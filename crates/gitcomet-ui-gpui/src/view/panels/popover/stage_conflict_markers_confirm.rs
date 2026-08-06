use super::*;

/// Warns that staging is about to mark files resolved while they still contain
/// conflict markers. Staging is what tells git a conflict is settled, so going
/// ahead here is how `<<<<<<<` ends up committed.
pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    paths: Vec<std::path::PathBuf>,
    unresolved: Vec<std::path::PathBuf>,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let listed = unresolved
        .iter()
        .take(5)
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let remaining = unresolved.len().saturating_sub(5);
    let detail = if remaining > 0 {
        format!("{listed}\n… and {remaining} more")
    } else {
        listed
    };
    let lead = if unresolved.len() == 1 {
        "This file still contains merge conflict markers:".to_string()
    } else {
        format!(
            "{} files still contain merge conflict markers:",
            unresolved.len()
        )
    };

    ConfirmDialog::new("Stage unresolved conflicts", DIALOG_420_WIDTH)
        .text(theme, lead)
        .text(theme, detail)
        .text(
            theme,
            "Staging marks a conflict resolved, so the markers would be committed as file content."
                .to_string(),
        )
        .render(
            theme,
            dialog_cancel_button(
                "stage_conflict_markers_cancel",
                "stage_conflict_markers_cancel_hint",
                theme,
                cx,
            ),
            components::Button::new("stage_conflict_markers_go", "Stage anyway")
                .style(components::ButtonStyle::Danger)
                .on_click(theme, cx, move |this, _e, _w, cx| {
                    this.store.dispatch(Msg::ClearDiffSelection { repo_id });
                    this.store.dispatch(Msg::StagePaths {
                        repo_id,
                        paths: paths.clone().into(),
                    });
                    this.close_popover(cx);
                }),
            cx,
        )
}
