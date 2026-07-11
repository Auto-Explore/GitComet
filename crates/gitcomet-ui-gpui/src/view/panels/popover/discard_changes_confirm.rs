use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    area: DiffArea,
    path: Option<std::path::PathBuf>,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let selected_paths_count = {
        let pane = this.details_pane.read(cx);
        pane.status_multi_selection
            .get(&repo_id)
            .map(|sel| sel.selected_count_for_area(area))
            .unwrap_or(0)
    };

    let (_count, detail, can_discard) = match path.as_ref() {
        Some(clicked_path) => {
            let (_use_selection, selected_count) = {
                let pane = this.details_pane.read(cx);
                let selection = pane
                    .status_multi_selection
                    .get(&repo_id)
                    .map(|sel| sel.selected_paths_for_area(area))
                    .unwrap_or(&[]);

                let use_selection =
                    selection.len() > 1 && selection.iter().any(|p| p == clicked_path);
                let selected_count = if use_selection { selection.len() } else { 1 };
                (use_selection, selected_count)
            };

            let detail = if selected_count == 1 {
                clicked_path.display().to_string()
            } else {
                format!("{selected_count} files")
            };
            (selected_count, detail, true)
        }
        None => {
            if selected_paths_count == 0 {
                (0, "No files selected.".to_string(), false)
            } else if selected_paths_count == 1 {
                let selected_path = this
                    .details_pane
                    .read(cx)
                    .status_multi_selection
                    .get(&repo_id)
                    .and_then(|sel| sel.first_selected_for_area(area))
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "file".to_string());
                (1, selected_path, true)
            } else {
                (
                    selected_paths_count,
                    format!("{selected_paths_count} files"),
                    true,
                )
            }
        }
    };

    ConfirmDialog::new("Discard changes", DIALOG_420_WIDTH)
        .text(
            theme,
            format!("This will discard working tree changes for {detail}."),
        )
        .render(
            theme,
            dialog_cancel_button(
                "discard_changes_cancel",
                "discard_changes_cancel_hint",
                theme,
                cx,
            ),
            components::Button::new("discard_changes_go", "Discard")
                .style(components::ButtonStyle::Danger)
                .disabled(!can_discard)
                .on_click(theme, cx, move |this, _e, _w, cx| {
                    this.discard_worktree_changes_confirmed(repo_id, area, path.clone(), cx);
                    this.close_popover(cx);
                }),
            cx,
        )
}
