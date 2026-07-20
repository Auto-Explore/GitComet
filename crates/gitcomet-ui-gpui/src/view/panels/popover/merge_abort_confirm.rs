use super::*;
use gitcomet_core::services::SequencerState;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    #[derive(Clone, Copy)]
    enum AbortMode {
        Merge,
        RebaseOrApply,
        CherryPick,
    }

    let mode = this
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .map(|repo| {
            if matches!(&repo.merge_commit_message, Loadable::Ready(Some(_))) {
                AbortMode::Merge
            } else if matches!(
                &repo.sequencer_state,
                Loadable::Ready(SequencerState::CherryPick)
            ) {
                AbortMode::CherryPick
            } else if matches!(&repo.rebase_in_progress, Loadable::Ready(true)) {
                AbortMode::RebaseOrApply
            } else {
                AbortMode::Merge
            }
        })
        .unwrap_or(AbortMode::Merge);

    let (title, body, command, button_id, button_label) = match mode {
        AbortMode::Merge => (
            "Abort merge?",
            "This will abort the current merge and restore the pre-merge state. Any resolved conflicts will be lost.",
            "git merge --abort",
            "merge_abort_go",
            "Abort merge",
        ),
        AbortMode::RebaseOrApply => (
            "Abort apply/rebase?",
            "This will abort the in-progress patch apply or rebase and restore the previous state. Any resolved conflicts will be lost.",
            "git rebase --abort / git am --abort",
            "rebase_or_apply_abort_go",
            "Abort",
        ),
        AbortMode::CherryPick => (
            "Abort cherry-pick?",
            "This will abort the current cherry-pick and restore the previous state. Any resolved conflicts will be lost.",
            "git cherry-pick --abort",
            "cherry_pick_abort_go",
            "Abort cherry-pick",
        ),
    };

    ConfirmDialog::new(title, DIALOG_360_WIDTH)
        .text(theme, body)
        .command(theme, command)
        .render(
            theme,
            dialog_cancel_button("merge_abort_cancel", "merge_abort_cancel_hint", theme, cx),
            components::Button::new(button_id, button_label)
                .style(components::ButtonStyle::Danger)
                .on_click(theme, cx, move |this, _e, _w, cx| {
                    match mode {
                        AbortMode::Merge => this.store.dispatch(Msg::MergeAbort { repo_id }),
                        AbortMode::RebaseOrApply => {
                            this.store.dispatch(Msg::RebaseAbort { repo_id })
                        }
                        AbortMode::CherryPick => this.store.dispatch(Msg::RebaseAbort { repo_id }),
                    }
                    this.close_popover(cx);
                }),
            cx,
        )
}
