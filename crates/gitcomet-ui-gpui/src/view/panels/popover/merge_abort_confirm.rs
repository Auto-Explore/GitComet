use super::*;

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
    }

    let mode = this
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .map(|repo| {
            if matches!(&repo.merge_commit_message, Loadable::Ready(Some(_))) {
                AbortMode::Merge
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
    };

    div()
        .flex()
        .flex_col()
        .min_w(px(360.0))
        .child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .child(title),
        )
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .text_color(theme.colors.text_muted)
                .child(body),
        )
        .child(
            div()
                .px_2()
                .pb_1()
                .text_xs()
                .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                .text_color(theme.colors.text_muted)
                .child(command),
        )
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    components::Button::new("merge_abort_cancel", "Cancel")
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.popover = None;
                            this.popover_anchor = None;
                            cx.notify();
                        }),
                )
                .child(
                    components::Button::new(button_id, button_label)
                        .style(components::ButtonStyle::Danger)
                        .on_click(theme, cx, move |this, _e, _w, cx| {
                            match mode {
                                AbortMode::Merge => {
                                    this.store.dispatch(Msg::MergeAbort { repo_id })
                                }
                                AbortMode::RebaseOrApply => {
                                    this.store.dispatch(Msg::RebaseAbort { repo_id })
                                }
                            }
                            this.popover = None;
                            this.popover_anchor = None;
                            cx.notify();
                        }),
                ),
        )
}
