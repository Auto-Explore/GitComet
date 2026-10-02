use super::*;

/// Discard for a status tree folder. The files are resolved when it is shown
/// and again on confirm, so a status refresh in between is honoured.
pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    section: StatusSection,
    folder: Arc<std::path::Path>,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let paths = this
        .details_pane
        .read(cx)
        .status_folder_subtree_paths(repo_id, section, &folder);
    let count = paths.len();
    let files = if count == 1 { "file" } else { "files" };
    let note = super::discard_changes_confirm::conflict_discard_note(
        &this.state,
        repo_id,
        paths.iter().map(|path| path.as_path()),
    );

    let mut dialog = ConfirmDialog::new("Discard changes", DIALOG_420_WIDTH)
        .text(
            theme,
            format!("This will discard working tree changes for {count} {files} in this folder."),
        )
        .mono_value(theme, folder.display().to_string());
    if let Some(note) = note {
        dialog = dialog.note(theme, note);
    }
    dialog.render(
        theme,
        dialog_cancel_button(
            "discard_folder_changes_cancel",
            "discard_folder_changes_cancel_hint",
            theme,
            cx,
        ),
        components::Button::new("discard_folder_changes_go", "Discard")
            .style(components::ButtonStyle::Danger)
            .disabled(count == 0)
            .on_click(theme, cx, move |this, _e, _w, cx| {
                let mut paths = this
                    .details_pane
                    .read(cx)
                    .status_folder_subtree_paths(repo_id, section, &folder);
                match paths.len() {
                    0 => {}
                    1 => this.store.dispatch(Msg::DiscardWorktreeChangesPath {
                        repo_id,
                        path: paths.remove(0),
                    }),
                    _ => this
                        .store
                        .dispatch(Msg::DiscardWorktreeChangesPaths { repo_id, paths }),
                }
                this.close_popover(cx);
            }),
        cx,
    )
}
