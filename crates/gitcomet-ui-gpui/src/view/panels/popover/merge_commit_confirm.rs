use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    commit_id: CommitId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let sha = commit_id.as_ref();
    let short: SharedString = sha.get(0..7).unwrap_or(sha).into();
    let summary = this
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .and_then(|repo| match &repo.log {
            Loadable::Ready(page) => page
                .commits
                .iter()
                .find(|commit| commit.id == commit_id)
                .map(|commit| commit.summary.to_string()),
            _ => None,
        })
        .unwrap_or_default();
    let current_branch: SharedString = this
        .active_repo()
        .and_then(|r| match &r.head_branch {
            Loadable::Ready(head) if !head.is_empty() && head != "HEAD" => {
                Some(head.as_str().into())
            }
            _ => None,
        })
        .unwrap_or_else(|| short.clone());

    let dispatch = move |this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>| {
        this.store.dispatch(Msg::MergeRef {
            repo_id,
            reference: commit_id.as_ref().to_string(),
        });
        this.close_popover(cx);
    };

    let mut dialog = ConfirmDialog::new("Merge commit?", DIALOG_380_WIDTH)
        .text(theme, format!("Merge {short} into {current_branch}?"))
        .note(
            theme,
            "Git resolves the merge with a merge commit when history diverges.",
        );
    if !summary.is_empty() {
        dialog = dialog.note(theme, summary);
    }

    dialog.render(
        theme,
        dialog_cancel_button("merge_commit_cancel", "merge_commit_cancel_hint", theme, cx),
        div().flex().items_center().gap_1().child(
            components::Button::new("merge_commit_confirm", "Merge")
                .style(components::ButtonStyle::Filled)
                .on_click(theme, cx, move |this, _e, _w, cx| dispatch(this, cx)),
        ),
        cx,
    )
}
