use super::*;
use gitcomet_core::services::SequencerState;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AbortMode {
    Merge,
    RebaseOrApply,
    CherryPick,
    Revert,
}

fn abort_mode(repo: &RepoState) -> AbortMode {
    if merge_in_progress(repo) {
        return AbortMode::Merge;
    }
    match active_sequencer_state(repo) {
        SequencerState::CherryPick => AbortMode::CherryPick,
        SequencerState::Revert => AbortMode::Revert,
        SequencerState::RebaseOrApply => AbortMode::RebaseOrApply,
        SequencerState::None => AbortMode::Merge,
    }
}

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let mode = this
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .map(abort_mode)
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
        AbortMode::Revert => (
            "Abort revert?",
            "This will stop the current revert and reset the working tree to HEAD. Any resolved conflicts and staged reverted changes will be lost; commits an earlier step already made stay on the branch.",
            "git revert --abort",
            "revert_abort_go",
            "Abort revert",
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
                        AbortMode::RebaseOrApply | AbortMode::CherryPick | AbortMode::Revert => {
                            this.store.dispatch(Msg::RebaseAbort { repo_id })
                        }
                    }
                    this.close_popover(cx);
                }),
            cx,
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::RepoSpec;
    use std::path::PathBuf;

    #[test]
    fn abort_mode_prefers_the_specific_sequencer_over_rebase_in_progress() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        );
        repo.rebase_in_progress = Loadable::Ready(true);
        for (state, expected) in [
            (SequencerState::Revert, AbortMode::Revert),
            (SequencerState::CherryPick, AbortMode::CherryPick),
            (SequencerState::RebaseOrApply, AbortMode::RebaseOrApply),
        ] {
            repo.sequencer_state = Loadable::Ready(state);
            assert_eq!(abort_mode(&repo), expected, "{state:?}");
        }

        repo.merge_commit_message = Loadable::Ready(Some("Merge branch 'topic'".to_string()));
        assert_eq!(abort_mode(&repo), AbortMode::Merge);
    }
}
