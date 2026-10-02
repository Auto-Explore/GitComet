use super::*;

/// Discarding a conflicted file keeps our side, so the dialogs say so.
pub(super) fn conflict_discard_note<'a>(
    state: &AppState,
    repo_id: RepoId,
    paths: impl IntoIterator<Item = &'a std::path::Path>,
) -> Option<&'static str> {
    let repo = state.repos.iter().find(|repo| repo.id == repo_id)?;
    // Conflicts are few, so collect them once rather than scanning per path.
    let conflicted: Vec<&std::path::Path> = repo
        .status_entries_for_area(DiffArea::Unstaged)?
        .iter()
        .filter(|entry| entry.kind == FileStatusKind::Conflicted)
        .map(|entry| entry.path.as_path())
        .collect();
    (!conflicted.is_empty() && paths.into_iter().any(|path| conflicted.contains(&path)))
        .then_some("Conflicted files keep your version (ours); the incoming change is dropped.")
}

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    area: DiffArea,
    path: Option<std::path::PathBuf>,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let pane = this.details_pane.read(cx);
    let selection = pane
        .status_multi_selection
        .get(&repo_id)
        .map(|sel| sel.selected_paths_for_area(area))
        .unwrap_or(&[]);

    // The clicked row counts on its own unless it is part of a larger selection.
    let targets: &[std::path::PathBuf] = match path.as_ref() {
        Some(clicked_path)
            if !(selection.len() > 1 && selection.iter().any(|p| p == clicked_path)) =>
        {
            std::slice::from_ref(clicked_path)
        }
        _ => selection,
    };
    let (detail, can_discard) = match targets {
        [] => ("No files selected.".to_string(), false),
        [single] => (single.display().to_string(), true),
        many => (format!("{} files", many.len()), true),
    };
    let note = conflict_discard_note(&this.state, repo_id, targets.iter().map(|p| p.as_path()));

    let mut dialog = ConfirmDialog::new("Discard changes", DIALOG_420_WIDTH).text(
        theme,
        format!("This will discard working tree changes for {detail}."),
    );
    if let Some(note) = note {
        dialog = dialog.note(theme, note);
    }
    dialog.render(
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
